use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::{
    environment::LaunchEnvironment,
    launch::LaunchConfig,
    session::{
        runtime_state_path, Session, SessionRuntimeState, GWT_BIN_PATH_ENV,
        GWT_HOOK_FORWARD_TOKEN_ENV, GWT_HOOK_FORWARD_URL_ENV, GWT_SESSION_ID_ENV,
        GWT_SESSION_RUNTIME_PATH_ENV,
    },
    types::{AgentId, DockerLifecycleIntent, LaunchRuntimeTarget},
};

const DOCKER_GWTD_BIN_PATH: &str = "/usr/local/bin/gwtd";
const DOCKER_HOST_GWT_BIN_NAME: &str = "gwt-linux";
const DOCKER_HOST_GWTD_BIN_NAME: &str = "gwtd-linux";
const DOCKER_GWT_OVERRIDE_HEADER: &str =
    "# Auto-generated docker-compose override for gwt bundle mounting";
const DOCKER_GWT_OVERRIDE_FILE_NAME: &str = "docker-compose.gwt.override.yml";
const DOCKER_USER_OVERRIDE_FILE_NAME: &str = "docker-compose.override.yml";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedProcessLaunch {
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub remove_env: Vec<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct PreparedAgentLaunch {
    pub process_launch: PreparedProcessLaunch,
    pub session: Session,
    pub runtime_path: PathBuf,
    pub worktree_path: PathBuf,
    pub used_host_package_runner_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HookForwardEnv {
    pub url: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DockerBundleMounts {
    host_gwt: PathBuf,
    host_gwtd: PathBuf,
}

#[derive(Debug, Clone)]
struct DockerLaunchPlan {
    compose_files: Vec<PathBuf>,
    service: String,
    container_cwd: String,
    target_arch: String,
}

impl DockerLaunchPlan {
    fn include_compose_override(&mut self, override_file: PathBuf) {
        if !self.compose_files.iter().any(|file| file == &override_file) {
            self.compose_files.push(override_file);
        }
    }
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
struct PackageRunnerProgram {
    executable: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, Default)]
struct DevContainerLaunchDefaults {
    service: Option<String>,
    workspace_folder: Option<String>,
    compose_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockerLaunchServiceAction {
    Connect,
    Start,
    Restart,
    Recreate,
}

type PackageRunnerProbe =
    dyn FnMut(&str, Vec<String>, &HashMap<String, String>, &[String], Option<PathBuf>) -> bool;
type GwtBinLookup = dyn Fn(&str) -> Option<PathBuf>;

struct PrepareLaunchDeps<'a> {
    current_exe: &'a Path,
    probe_host_runner: &'a mut PackageRunnerProbe,
    lookup_gwt_bin: &'a GwtBinLookup,
}

pub fn prepare_agent_launch<F>(
    repo_path: &Path,
    sessions_dir: &Path,
    config: LaunchConfig,
    hook_forward: Option<HookForwardEnv>,
    refresh_worktree_assets: F,
) -> Result<PreparedAgentLaunch, String>
where
    F: FnMut(&Path) -> Result<(), String>,
{
    let current_exe = std::env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    let mut probe_host_runner = probe_host_package_runner
        as fn(&str, Vec<String>, &HashMap<String, String>, &[String], Option<PathBuf>) -> bool;
    let lookup_gwt_bin = |command: &str| which::which(command).ok();
    prepare_agent_launch_with(
        repo_path,
        sessions_dir,
        config,
        hook_forward,
        refresh_worktree_assets,
        PrepareLaunchDeps {
            current_exe: &current_exe,
            probe_host_runner: &mut probe_host_runner,
            lookup_gwt_bin: &lookup_gwt_bin,
        },
    )
}

fn prepare_agent_launch_with<FRefresh>(
    repo_path: &Path,
    sessions_dir: &Path,
    mut config: LaunchConfig,
    hook_forward: Option<HookForwardEnv>,
    mut refresh_worktree_assets: FRefresh,
    deps: PrepareLaunchDeps<'_>,
) -> Result<PreparedAgentLaunch, String>
where
    FRefresh: FnMut(&Path) -> Result<(), String>,
{
    let PrepareLaunchDeps {
        current_exe,
        probe_host_runner,
        lookup_gwt_bin,
    } = deps;

    resolve_launch_worktree(repo_path, &mut config)?;
    apply_docker_runtime_to_launch_config(repo_path, &mut config)?;

    let worktree_path = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    LaunchEnvironment::empty()
        .with_project_root(&worktree_path)
        .apply_to_parts(&mut config.env_vars, &mut config.remove_env);
    refresh_worktree_assets(&worktree_path)?;

    let used_host_package_runner_fallback = config.runtime_target == LaunchRuntimeTarget::Host
        && apply_host_package_runner_fallback_with_probe(
            &mut config,
            "npx".to_string(),
            probe_host_runner,
        );

    install_launch_gwt_bin_env_with_lookup(
        &mut config.env_vars,
        config.runtime_target,
        current_exe,
        lookup_gwt_bin,
    )?;

    let branch_name = config
        .branch
        .clone()
        .unwrap_or_else(|| "workspace".to_string());
    let session = Session::from_launch_config(&worktree_path, branch_name, &config);
    let runtime_path = runtime_state_path(sessions_dir, &session.id);

    config
        .env_vars
        .insert(GWT_SESSION_ID_ENV.to_string(), session.id.clone());
    config.env_vars.insert(
        GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
        runtime_path.display().to_string(),
    );
    if let Some(target) = hook_forward {
        config
            .env_vars
            .insert(GWT_HOOK_FORWARD_URL_ENV.to_string(), target.url);
        config
            .env_vars
            .insert(GWT_HOOK_FORWARD_TOKEN_ENV.to_string(), target.token);
    }
    config
        .env_vars
        .entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());

    session
        .save(sessions_dir)
        .map_err(|error| error.to_string())?;
    SessionRuntimeState::new(crate::AgentStatus::Running)
        .save(&runtime_path)
        .map_err(|error| error.to_string())?;

    finalize_docker_agent_launch_config(repo_path, &mut config)?;

    Ok(PreparedAgentLaunch {
        process_launch: PreparedProcessLaunch {
            command: config.command,
            args: config.args,
            env: config.env_vars,
            remove_env: config.remove_env,
            cwd: config.working_dir,
        },
        session,
        runtime_path,
        worktree_path,
        used_host_package_runner_fallback,
    })
}

pub fn branch_worktree_path(repo_path: &Path, branch_name: &str) -> Option<PathBuf> {
    if current_git_branch(repo_path)
        .as_ref()
        .is_ok_and(|current| current == branch_name)
    {
        return Some(repo_path.to_path_buf());
    }

    let main_repo_path = gwt_git::worktree::main_worktree_root(repo_path).ok()?;
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    manager
        .list()
        .ok()?
        .into_iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch_name))
        .map(|worktree| worktree.path)
}

pub fn resolve_launch_worktree(repo_path: &Path, config: &mut LaunchConfig) -> Result<(), String> {
    resolve_launch_worktree_request(
        repo_path,
        config.branch.as_deref(),
        config.base_branch.as_deref(),
        &mut config.working_dir,
        &mut config.env_vars,
    )
}

pub fn resolve_launch_worktree_request(
    repo_path: &Path,
    branch_name: Option<&str>,
    base_branch: Option<&str>,
    working_dir: &mut Option<PathBuf>,
    env_vars: &mut HashMap<String, String>,
) -> Result<(), String> {
    let Some(branch_name) = branch_name.map(str::to_string) else {
        return Ok(());
    };
    if working_dir.is_some() {
        return Ok(());
    }

    let current_branch = current_git_branch(repo_path);
    if current_branch.is_err() && base_branch.is_none() {
        return Ok(());
    }
    if current_branch
        .as_ref()
        .is_ok_and(|current| current == &branch_name)
    {
        *working_dir = Some(repo_path.to_path_buf());
        env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            repo_path.display().to_string(),
        );
        return Ok(());
    }

    let main_repo_path =
        gwt_git::worktree::main_worktree_root(repo_path).map_err(|err| err.to_string())?;
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    let worktrees = manager.list().map_err(|err| err.to_string())?;
    if let Some(existing_worktree) = worktrees
        .iter()
        .find(|worktree| worktree.branch.as_deref() == Some(branch_name.as_str()))
        .map(|worktree| worktree.path.clone())
    {
        *working_dir = Some(existing_worktree.clone());
        env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            existing_worktree.display().to_string(),
        );
        return Ok(());
    }

    let base_branch = base_branch
        .map(str::to_string)
        .unwrap_or_else(|| "develop".to_string());
    let remote_base_ref = origin_remote_ref(&base_branch);
    let remote_branch_ref = origin_remote_ref(&branch_name);

    manager
        .fetch_origin()
        .map_err(|err| format!("failed to fetch origin: {err}"))?;

    if !manager
        .remote_branch_exists(&remote_base_ref)
        .map_err(|err| format!("failed to verify remote base branch {remote_base_ref}: {err}"))?
    {
        return Err(format!(
            "remote base branch does not exist: {remote_base_ref}"
        ));
    }

    if !manager
        .remote_branch_exists(&remote_branch_ref)
        .map_err(|err| format!("failed to verify remote branch {remote_branch_ref}: {err}"))?
    {
        manager
            .create_remote_branch_from_base(&remote_base_ref, &branch_name)
            .map_err(|err| {
                format!(
                    "failed to create remote branch {remote_branch_ref} from {remote_base_ref}: {err}"
                )
            })?;
        manager
            .fetch_origin()
            .map_err(|err| format!("failed to refresh origin refs after push: {err}"))?;
    }

    let preferred_worktree_path =
        gwt_git::worktree::sibling_worktree_path(&main_repo_path, &branch_name);
    let worktree_path = first_available_worktree_path(&preferred_worktree_path, &worktrees)
        .ok_or_else(|| {
            format!("failed to resolve available worktree path for branch {branch_name}")
        })?;
    if local_branch_exists(&main_repo_path, &branch_name)? {
        manager
            .create(&branch_name, &worktree_path)
            .map_err(|err| err.to_string())?;
    } else {
        manager
            .create_from_remote(&remote_branch_ref, &branch_name, &worktree_path)
            .map_err(|err| err.to_string())?;
    }

    *working_dir = Some(worktree_path.clone());
    env_vars.insert(
        "GWT_PROJECT_ROOT".to_string(),
        worktree_path.display().to_string(),
    );
    Ok(())
}

pub fn apply_host_package_runner_fallback(config: &mut LaunchConfig) -> bool {
    apply_host_package_runner_fallback_with_probe(
        config,
        "npx".to_string(),
        probe_host_package_runner,
    )
}

pub fn apply_host_package_runner_fallback_with_probe<F>(
    config: &mut LaunchConfig,
    fallback_executable: String,
    mut probe: F,
) -> bool
where
    F: FnMut(&str, Vec<String>, &HashMap<String, String>, &[String], Option<PathBuf>) -> bool,
{
    let Some(program) =
        resolve_host_package_runner_with_probe(config, fallback_executable, &mut probe)
    else {
        return false;
    };
    config.command = program.executable;
    config.args = program.args;
    true
}

pub fn install_launch_gwt_bin_env(
    env_vars: &mut HashMap<String, String>,
    runtime_target: LaunchRuntimeTarget,
) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    install_launch_gwt_bin_env_with_lookup(env_vars, runtime_target, &current_exe, |command| {
        which::which(command).ok()
    })
}

pub fn install_launch_gwt_bin_env_with_lookup(
    env_vars: &mut HashMap<String, String>,
    runtime_target: LaunchRuntimeTarget,
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<(), String> {
    let gwt_bin = match runtime_target {
        LaunchRuntimeTarget::Docker => DOCKER_GWTD_BIN_PATH.to_string(),
        LaunchRuntimeTarget::Host => resolve_public_gwt_bin_with_lookup(current_exe, lookup)
            .to_string_lossy()
            .into_owned(),
    };
    match runtime_target {
        LaunchRuntimeTarget::Docker => {
            env_vars.insert(GWT_BIN_PATH_ENV.to_string(), gwt_bin);
        }
        LaunchRuntimeTarget::Host => {
            env_vars
                .entry(GWT_BIN_PATH_ENV.to_string())
                .or_insert(gwt_bin);
        }
    }
    Ok(())
}

pub fn resolve_public_gwt_bin_with_lookup(
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> PathBuf {
    if should_prefer_path_gwt(current_exe) {
        if let Some(candidate) = lookup("gwtd").filter(|candidate| {
            !same_path(candidate, current_exe) && !is_bunx_temp_executable(candidate)
        }) {
            return candidate;
        }
        if let Some(candidate) = sibling_gwtd_binary(current_exe) {
            return candidate;
        }
    }
    current_exe.to_path_buf()
}

fn sibling_gwtd_binary(path: &Path) -> Option<PathBuf> {
    if !is_named_gwt_binary(path) {
        return None;
    }
    let sibling_name = match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("exe") => "gwtd.exe".to_string(),
        _ => "gwtd".to_string(),
    };
    Some(path.with_file_name(sibling_name))
}

fn apply_docker_runtime_to_launch_config(
    repo_path: &Path,
    config: &mut LaunchConfig,
) -> Result<(), String> {
    if config.runtime_target != LaunchRuntimeTarget::Docker {
        return Ok(());
    }

    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    ensure_docker_launch_runtime_ready()?;
    let mut launch = launch;
    let compose_override_file =
        ensure_docker_gwt_binary_setup(&worktree, &launch.service, &launch.target_arch)?;
    launch.include_compose_override(compose_override_file);
    ensure_docker_launch_service_ready(&launch, config.docker_lifecycle_intent)?;
    maybe_inject_docker_sandbox_env(&launch, config)?;
    install_launch_gwt_bin_env(&mut config.env_vars, LaunchRuntimeTarget::Docker)?;
    let runtime_program = resolve_docker_exec_program(&launch, config)?;
    config.command = runtime_program.executable;
    config.args = runtime_program.args;
    config
        .env_vars
        .insert("GWT_PROJECT_ROOT".to_string(), launch.container_cwd.clone());
    config.docker_service = Some(launch.service);
    Ok(())
}

fn finalize_docker_agent_launch_config(
    repo_path: &Path,
    config: &mut LaunchConfig,
) -> Result<(), String> {
    if config.runtime_target != LaunchRuntimeTarget::Docker {
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

    let mut args = docker_compose_command_prefix(&launch);
    args.extend(["exec".to_string(), "-w".to_string(), launch.container_cwd]);
    args.extend(docker_compose_exec_env_args(&config.env_vars));
    args.push(launch.service);
    args.push(runtime_program.executable);
    args.extend(runtime_program.args);

    config.command = docker_binary_for_launch();
    config.args = args;
    Ok(())
}

fn resolve_host_package_runner_with_probe<F>(
    config: &LaunchConfig,
    fallback_executable: String,
    probe: &mut F,
) -> Option<PackageRunnerProgram>
where
    F: FnMut(&str, Vec<String>, &HashMap<String, String>, &[String], Option<PathBuf>) -> bool,
{
    let version_spec = host_package_runner_version_spec(config)?;
    if !command_matches_runner(&config.command, "bunx") {
        return None;
    }

    let probe_args = vec![version_spec.clone(), "--version".to_string()];
    let cwd = config.working_dir.clone();
    if probe(
        &config.command,
        probe_args,
        &config.env_vars,
        &config.remove_env,
        cwd,
    ) {
        return None;
    }

    let agent_args = strip_package_runner_args(&config.args, &version_spec);
    let mut args = vec!["--yes".to_string(), version_spec];
    args.extend(agent_args);
    Some(PackageRunnerProgram {
        executable: fallback_executable,
        args,
    })
}

fn host_package_runner_version_spec(config: &LaunchConfig) -> Option<String> {
    package_runner_version_spec(config)
        .or_else(|| infer_package_runner_version_spec(&config.command, &config.args))
}

fn infer_package_runner_version_spec(command: &str, args: &[String]) -> Option<String> {
    if !(command_matches_runner(command, "bunx") || command_matches_runner(command, "npx")) {
        return None;
    }

    let version_spec = match args.first().map(String::as_str) {
        Some("--yes") | Some("-y") => args.get(1)?,
        _ => args.first()?,
    };
    if version_spec.is_empty() || version_spec.starts_with('-') {
        return None;
    }
    Some(version_spec.clone())
}

fn probe_host_package_runner(
    command: &str,
    args: Vec<String>,
    env_vars: &HashMap<String, String>,
    remove_env: &[String],
    cwd: Option<PathBuf>,
) -> bool {
    let mut process = Command::new(command);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    for key in remove_env {
        process.env_remove(key);
    }
    process.envs(env_vars);
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }
    process.status().is_ok_and(|status| status.success())
}

fn command_matches_runner(command: &str, runner: &str) -> bool {
    let path = Path::new(command);
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .is_some_and(|name| name.eq_ignore_ascii_case(runner))
}

fn ensure_docker_launch_runtime_ready() -> Result<(), String> {
    if !gwt_docker::docker_available() {
        return Err("Docker is not installed or not available on PATH".to_string());
    }
    if !gwt_docker::compose_available() {
        return Err("docker compose is not available".to_string());
    }
    if !gwt_docker::daemon_running() {
        return Err("Docker daemon is not running".to_string());
    }
    Ok(())
}

fn docker_bundle_mounts_for_gwt_home(gwt_home: &Path) -> DockerBundleMounts {
    let gwt_bin_dir = gwt_home.join("bin");
    DockerBundleMounts {
        host_gwt: gwt_bin_dir.join(DOCKER_HOST_GWT_BIN_NAME),
        host_gwtd: gwt_bin_dir.join(DOCKER_HOST_GWTD_BIN_NAME),
    }
}

#[cfg(test)]
fn docker_bundle_mounts_for_home(home: &Path) -> DockerBundleMounts {
    docker_bundle_mounts_for_gwt_home(&home.join(".gwt"))
}

fn docker_compose_mount_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn docker_bundle_override_content(service: &str, bundle: &DockerBundleMounts) -> String {
    let host_gwtd = docker_compose_mount_path(&bundle.host_gwtd);
    format!(
        concat!(
            "{header}\n",
            "version: '3.8'\n",
            "services:\n",
            "  {service}:\n",
            "    volumes:\n",
            "      - \"{host_gwtd}:{path}:ro\"\n"
        ),
        header = DOCKER_GWT_OVERRIDE_HEADER,
        service = service,
        host_gwtd = host_gwtd,
        path = DOCKER_GWTD_BIN_PATH,
    )
}

fn ensure_docker_gwt_binary_setup(
    repo_path: &Path,
    service: &str,
    target_arch: &str,
) -> Result<PathBuf, String> {
    let gwt_home = gwt_core::paths::gwt_home();
    ensure_docker_gwt_binary_setup_for_gwt_home(repo_path, service, &gwt_home, |bundle| {
        eprintln!(
            "Installing Linux gwt bundle for Docker at {} and {}",
            bundle.host_gwt.display(),
            bundle.host_gwtd.display()
        );
        let installed = gwt_core::update::UpdateManager::new().install_latest_docker_linux_bundle(
            target_arch,
            &bundle.host_gwt,
            &bundle.host_gwtd,
        )?;
        eprintln!(
            "Installed Linux gwt bundle v{} for Docker",
            installed.version
        );
        Ok(())
    })
}

fn docker_compose_override_path(repo_path: &Path) -> PathBuf {
    repo_path.join(DOCKER_GWT_OVERRIDE_FILE_NAME)
}

fn docker_compose_user_override_path(repo_path: &Path) -> PathBuf {
    repo_path.join(DOCKER_USER_OVERRIDE_FILE_NAME)
}

fn is_legacy_gwt_generated_override(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .is_ok_and(|content| content.starts_with(DOCKER_GWT_OVERRIDE_HEADER))
}

#[cfg(test)]
fn ensure_docker_gwt_binary_setup_for_home<F>(
    repo_path: &Path,
    service: &str,
    home: &Path,
    install_bundle: F,
) -> Result<PathBuf, String>
where
    F: FnMut(&DockerBundleMounts) -> Result<(), String>,
{
    let gwt_home = home.join(".gwt");
    ensure_docker_gwt_binary_setup_for_gwt_home(repo_path, service, &gwt_home, install_bundle)
}

fn ensure_docker_gwt_binary_setup_for_gwt_home<F>(
    repo_path: &Path,
    service: &str,
    gwt_home: &Path,
    mut install_bundle: F,
) -> Result<PathBuf, String>
where
    F: FnMut(&DockerBundleMounts) -> Result<(), String>,
{
    use std::fs;

    let bundle = docker_bundle_mounts_for_gwt_home(gwt_home);

    if !docker_bundle_binary_ready(&bundle.host_gwt)
        || !docker_bundle_binary_ready(&bundle.host_gwtd)
    {
        install_bundle(&bundle).map_err(|err| {
            format!(
                "Failed to install Linux gwt bundle for Docker: {err}\n\
                 Expected cached binaries at {} and {}",
                bundle.host_gwt.display(),
                bundle.host_gwtd.display()
            )
        })?;
    }

    if !docker_bundle_binary_ready(&bundle.host_gwt)
        || !docker_bundle_binary_ready(&bundle.host_gwtd)
    {
        return Err(format!(
            "Linux gwt bundle setup did not create expected Docker binaries at {} and {}",
            bundle.host_gwt.display(),
            bundle.host_gwtd.display()
        ));
    }

    let override_path = docker_compose_override_path(repo_path);
    let override_content = docker_bundle_override_content(service, &bundle);
    let rewrite_override = fs::read_to_string(&override_path)
        .map(|existing| existing != override_content)
        .unwrap_or(true);
    if rewrite_override {
        fs::write(&override_path, override_content).map_err(|err| {
            format!(
                "Failed to write generated Docker compose override: {err}\n\
                 Manually create {} with gwt/gwtd bundle mounts",
                override_path.display()
            )
        })?;
    }

    Ok(override_path)
}

fn docker_bundle_binary_ready(path: &Path) -> bool {
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
}

fn maybe_inject_docker_sandbox_env(
    launch: &DockerLaunchPlan,
    config: &mut LaunchConfig,
) -> Result<(), String> {
    if cfg!(windows) || !matches!(config.agent_id, AgentId::ClaudeCode) || !config.skip_permissions
    {
        return Ok(());
    }

    let is_root =
        gwt_docker::compose_service_user_is_root_with_files(&launch.compose_files, &launch.service)
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

fn docker_compose_exec_env_args(env_vars: &HashMap<String, String>) -> Vec<String> {
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

fn is_valid_docker_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn resolve_docker_exec_program(
    launch: &DockerLaunchPlan,
    config: &LaunchConfig,
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

fn package_runner_version_spec(config: &LaunchConfig) -> Option<String> {
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
    config: &LaunchConfig,
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
        let output = gwt_docker::compose_service_exec_capture_with_files(
            &launch.compose_files,
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

fn strip_package_runner_args(args: &[String], version_spec: &str) -> Vec<String> {
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

fn ensure_docker_launch_command_ready(
    launch: &DockerLaunchPlan,
    command: &str,
) -> Result<(), String> {
    let available = gwt_docker::compose_service_has_command_with_files(
        &launch.compose_files,
        &launch.service,
        command,
    )
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

fn ensure_docker_launch_service_ready(
    launch: &DockerLaunchPlan,
    intent: DockerLifecycleIntent,
) -> Result<(), String> {
    let status =
        gwt_docker::compose_service_status_with_files(&launch.compose_files, &launch.service)
            .map_err(|err| err.to_string())?;
    match normalize_docker_launch_action(intent, status) {
        DockerLaunchServiceAction::Connect => Ok(()),
        DockerLaunchServiceAction::Start => {
            gwt_docker::compose_up_with_files(&launch.compose_files, &launch.service)
                .map_err(|err| err.to_string())?;
            Ok(())
        }
        DockerLaunchServiceAction::Restart => {
            gwt_docker::compose_restart_with_files(&launch.compose_files, &launch.service)
                .map_err(|err| err.to_string())
        }
        DockerLaunchServiceAction::Recreate => {
            gwt_docker::compose_up_force_recreate_with_files(&launch.compose_files, &launch.service)
                .map_err(|err| err.to_string())
        }
    }
}

fn normalize_docker_launch_action(
    intent: DockerLifecycleIntent,
    status: gwt_docker::ComposeServiceStatus,
) -> DockerLaunchServiceAction {
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
            ComposeServiceStatus::Unknown
            | ComposeServiceStatus::Stopped
            | ComposeServiceStatus::Exited
            | ComposeServiceStatus::NotFound => DockerLaunchServiceAction::Start,
        },
    }
}

fn resolve_docker_launch_plan(
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
        compose_files: docker_launch_compose_files(worktree, &compose_file),
        service: service.name.clone(),
        container_cwd,
        target_arch: docker_bundle_target_arch(service)?,
    })
}

fn docker_binary_for_launch() -> String {
    std::env::var("GWT_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string())
}

fn docker_bundle_target_arch(service: &gwt_docker::ComposeService) -> Result<String, String> {
    if let Some(platform) = service.platform.as_deref() {
        return docker_platform_target_arch(platform).ok_or_else(|| {
            format!(
                "Docker service {} specifies unsupported platform {}; expected x86_64/amd64 or aarch64/arm64",
                service.name, platform
            )
        });
    }
    Ok(host_docker_target_arch())
}

fn docker_platform_target_arch(platform: &str) -> Option<String> {
    let platform = platform.trim();
    let arch = platform
        .split('/')
        .nth(1)
        .filter(|value| !value.is_empty())
        .unwrap_or(platform);
    normalize_docker_target_arch(arch)
}

fn host_docker_target_arch() -> String {
    normalize_docker_target_arch(std::env::consts::ARCH)
        .unwrap_or_else(|| std::env::consts::ARCH.to_string())
}

fn normalize_docker_target_arch(raw: &str) -> Option<String> {
    match raw
        .trim()
        .to_ascii_lowercase()
        .split('/')
        .next()
        .unwrap_or_default()
    {
        "x86_64" | "amd64" | "x64" => Some("x86_64".to_string()),
        "aarch64" | "arm64" => Some("aarch64".to_string()),
        _ => None,
    }
}

fn docker_launch_compose_files(worktree: &Path, compose_file: &Path) -> Vec<PathBuf> {
    let mut files = vec![compose_file.to_path_buf()];
    let user_override_file = docker_compose_user_override_path(worktree);
    if user_override_file.is_file() && !is_legacy_gwt_generated_override(&user_override_file) {
        files.push(user_override_file);
    }
    let generated_override_file = docker_compose_override_path(worktree);
    if generated_override_file.is_file() {
        files.push(generated_override_file);
    }
    files
}

fn docker_compose_command_prefix(launch: &DockerLaunchPlan) -> Vec<String> {
    let mut args = vec!["compose".to_string()];
    for compose_file in &launch.compose_files {
        args.push("-f".to_string());
        args.push(compose_file.display().to_string());
    }
    args
}

fn docker_compose_file_for_launch(
    project_root: &Path,
    files: &gwt_docker::DockerFiles,
) -> Result<Option<PathBuf>, String> {
    Ok(docker_devcontainer_defaults(project_root, files)
        .and_then(|defaults| defaults.compose_file)
        .or_else(|| files.compose_file.clone()))
}

fn docker_devcontainer_defaults(
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

fn compose_workspace_mount_target(
    project_root: &Path,
    service: &gwt_docker::ComposeService,
) -> Option<String> {
    service
        .volumes
        .iter()
        .find(|mount| mount_source_matches_project_root(&mount.source, project_root))
        .map(|mount| mount.target.clone())
}

fn mount_source_matches_project_root(source: &str, project_root: &Path) -> bool {
    let normalized = source
        .trim()
        .trim_end_matches(['/', '\\'])
        .trim_end_matches("/.");

    if matches!(normalized, "." | "$PWD" | "${PWD}") {
        return true;
    }

    let source_path = Path::new(normalized);
    source_path.is_absolute() && same_path(source_path, project_root)
}

fn first_available_worktree_path(
    preferred_path: &Path,
    worktrees: &[gwt_git::WorktreeInfo],
) -> Option<PathBuf> {
    if !worktree_path_is_occupied(preferred_path, worktrees) && !preferred_path.exists() {
        return Some(preferred_path.to_path_buf());
    }

    for suffix in 2usize.. {
        let candidate = suffixed_worktree_path(preferred_path, suffix)?;
        if !worktree_path_is_occupied(&candidate, worktrees) && !candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn suffixed_worktree_path(path: &Path, suffix: usize) -> Option<PathBuf> {
    let file_name = path.file_name()?.to_str()?;
    let mut candidate = path.to_path_buf();
    candidate.set_file_name(format!("{file_name}-{suffix}"));
    Some(candidate)
}

fn worktree_path_is_occupied(path: &Path, worktrees: &[gwt_git::WorktreeInfo]) -> bool {
    worktrees
        .iter()
        .any(|worktree| same_path(&worktree.path, path))
}

fn origin_remote_ref(branch_name: &str) -> String {
    if let Some(ref_name) = branch_name.strip_prefix("refs/remotes/") {
        ref_name.to_string()
    } else if branch_name.starts_with("origin/") {
        branch_name.to_string()
    } else {
        format!("origin/{branch_name}")
    }
}

fn current_git_branch(repo_path: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .args(["branch", "--show-current"])
        .current_dir(repo_path)
        .output()
        .map_err(|err| format!("git branch --show-current: {err}"))?;
    if !output.status.success() {
        return Err(format!(
            "git branch --show-current: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let branch = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if branch.is_empty() {
        return Err("git branch --show-current: detached HEAD".to_string());
    }
    Ok(branch)
}

fn local_branch_exists(repo_path: &Path, branch_name: &str) -> Result<bool, String> {
    let output = Command::new("git")
        .args(["show-ref", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch_name}"))
        .current_dir(repo_path)
        .status()
        .map_err(|err| format!("git show-ref --verify refs/heads/{branch_name}: {err}"))?;
    Ok(output.success())
}

fn should_prefer_path_gwt(current_exe: &Path) -> bool {
    is_bunx_temp_executable(current_exe) || !is_named_gwtd_binary(current_exe)
}

fn is_named_gwt_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| value.trim_end_matches(".exe").to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case("gwt"))
}

fn is_named_gwtd_binary(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .next_back()
        .map(|value| value.trim_end_matches(".exe").to_string())
        .is_some_and(|value| value.eq_ignore_ascii_case("gwtd"))
}

fn is_bunx_temp_executable(path: &Path) -> bool {
    normalized_path_segments(path)
        .into_iter()
        .any(|segment| segment.starts_with("bunx-"))
}

fn normalized_path_segments(path: &Path) -> Vec<String> {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_string)
        .collect()
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }

    let left = dunce::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = dunce::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    left == right
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentLaunchBuilder, SessionMode};
    use std::{
        fs,
        process::Command,
        sync::atomic::{AtomicUsize, Ordering},
    };
    use tempfile::tempdir;

    fn sample_versioned_launch_config(worktree: &Path) -> LaunchConfig {
        let mut config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .working_dir(worktree)
            .branch("feature/demo")
            .version("latest")
            .session_mode(SessionMode::Normal)
            .build();
        config.command = "bunx".to_string();
        config.args = vec![
            "@anthropic-ai/claude-code@latest".to_string(),
            "--print".to_string(),
        ];
        config.env_vars = HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]);
        config.working_dir = Some(worktree.to_path_buf());
        config.runtime_target = LaunchRuntimeTarget::Host;
        config.docker_lifecycle_intent = DockerLifecycleIntent::Connect;
        config
    }

    fn sample_custom_bunx_launch_config(worktree: &Path) -> LaunchConfig {
        let mut config = AgentLaunchBuilder::new(AgentId::Custom("claude-code-openai".to_string()))
            .working_dir(worktree)
            .branch("feature/demo")
            .session_mode(SessionMode::Normal)
            .build();
        config.command = "bunx".to_string();
        config.args = vec![
            "@anthropic-ai/claude-code@latest".to_string(),
            "--print".to_string(),
        ];
        config.env_vars = HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]);
        config.working_dir = Some(worktree.to_path_buf());
        config.runtime_target = LaunchRuntimeTarget::Host;
        config.docker_lifecycle_intent = DockerLifecycleIntent::Connect;
        config
    }

    #[test]
    fn host_package_runner_version_spec_uses_runner_args_for_custom_bunx_launch() {
        let temp = tempdir().expect("tempdir");
        let config = sample_custom_bunx_launch_config(temp.path());

        assert_eq!(super::package_runner_version_spec(&config), None);
        assert_eq!(
            super::host_package_runner_version_spec(&config),
            Some("@anthropic-ai/claude-code@latest".to_string())
        );
    }

    #[test]
    fn prepare_agent_launch_persists_session_and_builds_process_launch() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join(".gwt").join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");

        let refresh_calls = AtomicUsize::new(0);
        let mut config = sample_versioned_launch_config(&worktree);
        config
            .env_vars
            .insert("GWT_PROJECT_ROOT".to_string(), "/stale/project".to_string());
        let expected_project_root = worktree.display().to_string();
        let probe_expected_project_root = expected_project_root.clone();
        let mut probe_host_runner =
            move |_command: &str,
                  _args: Vec<String>,
                  env: &HashMap<String, String>,
                  _remove_env: &[String],
                  _cwd: Option<PathBuf>| {
                assert_eq!(
                    env.get("GWT_PROJECT_ROOT").map(String::as_str),
                    Some(probe_expected_project_root.as_str())
                );
                false
            };
        let lookup_gwt_bin =
            |_command: &str| Some(PathBuf::from(r"C:\Users\Example\.bun\bin\gwtd.exe"));
        let prepared = prepare_agent_launch_with(
            &worktree,
            &sessions_dir,
            config,
            Some(HookForwardEnv {
                url: "http://127.0.0.1:7878/hooks".to_string(),
                token: "secret-token".to_string(),
            }),
            |path| {
                assert_eq!(path, worktree.as_path());
                refresh_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            PrepareLaunchDeps {
                current_exe: Path::new(
                    r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
                ),
                probe_host_runner: &mut probe_host_runner,
                lookup_gwt_bin: &lookup_gwt_bin,
            },
        )
        .expect("prepare launch");

        assert_eq!(refresh_calls.load(Ordering::SeqCst), 1);
        assert!(prepared.used_host_package_runner_fallback);
        assert_eq!(prepared.process_launch.command, "npx");
        assert_eq!(
            prepared.process_launch.cwd.as_deref(),
            Some(worktree.as_path())
        );
        assert_eq!(
            prepared
                .process_launch
                .env
                .get("GWT_PROJECT_ROOT")
                .map(String::as_str),
            Some(expected_project_root.as_str())
        );
        assert_eq!(
            prepared
                .process_launch
                .env
                .get(GWT_BIN_PATH_ENV)
                .map(String::as_str),
            Some(r"C:\Users\Example\.bun\bin\gwtd.exe")
        );
        assert_eq!(
            prepared
                .process_launch
                .env
                .get(GWT_HOOK_FORWARD_URL_ENV)
                .map(String::as_str),
            Some("http://127.0.0.1:7878/hooks")
        );
        assert_eq!(
            prepared
                .process_launch
                .env
                .get(GWT_HOOK_FORWARD_TOKEN_ENV)
                .map(String::as_str),
            Some("secret-token")
        );
        assert!(prepared.runtime_path.exists());
        assert!(sessions_dir
            .join(format!("{}.toml", prepared.session.id))
            .exists());
        assert_eq!(prepared.session.launch_command, "npx");
        assert_eq!(prepared.session.branch, "feature/demo");
    }

    #[test]
    fn prepare_agent_launch_uses_npx_fallback_for_custom_bunx_launch() {
        let temp = tempdir().expect("tempdir");
        let worktree = temp.path().join("repo-feature");
        let sessions_dir = temp.path().join(".gwt").join("sessions");
        fs::create_dir_all(&worktree).expect("create worktree");

        let mut probe_host_runner =
            |_command: &str,
             _args: Vec<String>,
             _env: &HashMap<String, String>,
             _remove_env: &[String],
             _cwd: Option<PathBuf>| false;
        let lookup_gwt_bin =
            |_command: &str| Some(PathBuf::from(r"C:\Users\Example\.bun\bin\gwt.exe"));
        let prepared = prepare_agent_launch_with(
            &worktree,
            &sessions_dir,
            sample_custom_bunx_launch_config(&worktree),
            None,
            |path| {
                assert_eq!(path, worktree.as_path());
                Ok(())
            },
            PrepareLaunchDeps {
                current_exe: Path::new(
                    r"C:\Users\Example\AppData\Local\Temp\bunx-1234567890-@akiojin\gwt@latest\node_modules\@akiojin\gwt\bin\gwt.exe",
                ),
                probe_host_runner: &mut probe_host_runner,
                lookup_gwt_bin: &lookup_gwt_bin,
            },
        )
        .expect("prepare launch");

        assert!(prepared.used_host_package_runner_fallback);
        assert_eq!(prepared.process_launch.command, "npx");
        assert_eq!(
            prepared.process_launch.args,
            vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
                "--print".to_string(),
            ]
        );
        assert_eq!(prepared.session.launch_command, "npx");
        assert_eq!(
            prepared.session.launch_args,
            vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
                "--print".to_string(),
            ]
        );
    }

    #[test]
    fn docker_bundle_override_content_mounts_gwtd_only_for_agents() {
        let home = PathBuf::from("/home/example");
        let bundle = docker_bundle_mounts_for_home(&home);
        let content = docker_bundle_override_content("app", &bundle);

        assert!(content.contains("/home/example/.gwt/bin/gwtd-linux:/usr/local/bin/gwtd:ro"));
        assert!(!content.contains("/usr/local/bin/gwt:ro"));
        assert!(!content.contains("gwtd-linux:/usr/local/bin/gwt:ro"));
        let volume_lines = content
            .lines()
            .filter(|line| line.contains(":/usr/local/bin/gwtd:ro"))
            .collect::<Vec<_>>();
        assert_eq!(volume_lines.len(), 1);
        assert!(volume_lines
            .iter()
            .all(|line| line.trim_start().starts_with("- ")));

        let parsed: serde_yaml::Value =
            serde_yaml::from_str(&content).expect("override must parse as YAML");
        let services = parsed
            .get("services")
            .and_then(|v| v.as_mapping())
            .expect("services key must be a YAML mapping");
        let service_def = services
            .get(serde_yaml::Value::String("app".to_string()))
            .and_then(|v| v.as_mapping())
            .expect("service entry must be a mapping");
        let volumes = service_def
            .get(serde_yaml::Value::String("volumes".to_string()))
            .and_then(|v| v.as_sequence())
            .expect("volumes must be a sequence");
        assert_eq!(volumes.len(), 1);
    }

    #[test]
    fn docker_binary_setup_installs_missing_bundle_before_writing_override() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let mut installer_calls = 0;

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), |bundle| {
            installer_calls += 1;
            fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent"))
                .expect("create bin dir");
            fs::write(&bundle.host_gwt, b"linux-gwt").expect("write gwt");
            fs::write(&bundle.host_gwtd, b"linux-gwtd").expect("write gwtd");
            Ok(())
        })
        .expect("docker setup");

        let bundle = docker_bundle_mounts_for_home(home.path());
        assert_eq!(installer_calls, 1);
        assert_eq!(fs::read(&bundle.host_gwt).expect("read gwt"), b"linux-gwt");
        assert_eq!(
            fs::read(&bundle.host_gwtd).expect("read gwtd"),
            b"linux-gwtd"
        );

        let override_content = fs::read_to_string(docker_compose_override_path(repo.path()))
            .expect("override content");
        assert!(override_content.contains("gwtd-linux:/usr/local/bin/gwtd:ro"));
        assert!(!override_content.contains("/usr/local/bin/gwt:ro"));
    }

    #[test]
    fn docker_binary_setup_repairs_directory_placeholders_before_writing_override() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        fs::create_dir_all(&bundle.host_gwt).expect("create gwt placeholder dir");
        fs::create_dir_all(&bundle.host_gwtd).expect("create gwtd placeholder dir");
        let mut installer_calls = 0;

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), |bundle| {
            installer_calls += 1;
            if bundle.host_gwt.is_dir() {
                fs::remove_dir_all(&bundle.host_gwt).expect("remove gwt placeholder");
            }
            if bundle.host_gwtd.is_dir() {
                fs::remove_dir_all(&bundle.host_gwtd).expect("remove gwtd placeholder");
            }
            fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent"))
                .expect("create bin dir");
            fs::write(&bundle.host_gwt, b"linux-gwt").expect("write gwt");
            fs::write(&bundle.host_gwtd, b"linux-gwtd").expect("write gwtd");
            Ok(())
        })
        .expect("docker setup");

        assert_eq!(installer_calls, 1);
        assert!(bundle.host_gwt.is_file());
        assert!(bundle.host_gwtd.is_file());
        assert!(docker_compose_override_path(repo.path()).is_file());
    }

    #[test]
    fn docker_binary_setup_skips_installer_when_bundle_exists() {
        let repo = tempdir().expect("repo tempdir");
        let home = tempdir().expect("home tempdir");
        let bundle = docker_bundle_mounts_for_home(home.path());
        fs::create_dir_all(bundle.host_gwt.parent().expect("gwt parent")).expect("create bin dir");
        fs::write(&bundle.host_gwt, b"existing-gwt").expect("write gwt");
        fs::write(&bundle.host_gwtd, b"existing-gwtd").expect("write gwtd");

        ensure_docker_gwt_binary_setup_for_home(repo.path(), "app", home.path(), |_| {
            panic!("installer should not run when both bundle binaries exist");
        })
        .expect("docker setup");

        assert!(docker_compose_override_path(repo.path()).is_file());
    }

    #[test]
    fn docker_launch_compose_files_skips_legacy_generated_default_override() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("repo");
        fs::create_dir_all(&project).expect("project dir");
        let compose_file = project.join("docker-compose.yml");
        fs::write(&compose_file, "services: {}\n").expect("compose file");
        fs::write(
            docker_compose_user_override_path(&project),
            format!("{DOCKER_GWT_OVERRIDE_HEADER}\nservices: {{}}\n"),
        )
        .expect("legacy default override");

        assert_eq!(
            docker_launch_compose_files(&project, &compose_file),
            vec![compose_file]
        );
    }

    #[test]
    fn finalize_docker_agent_launch_config_wraps_compose_exec() {
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        fs::create_dir_all(&project).expect("create project");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/app\n",
        )
        .expect("write compose file");

        let mut config = AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir(&project)
            .build();
        config.runtime_target = LaunchRuntimeTarget::Docker;
        config.docker_service = Some("app".to_string());
        config.command = "codex".to_string();
        config.args = vec!["--no-alt-screen".to_string()];
        config.env_vars = HashMap::from([
            (GWT_SESSION_ID_ENV.to_string(), "sess-123".to_string()),
            (
                GWT_SESSION_RUNTIME_PATH_ENV.to_string(),
                "/tmp/runtime/sess-123.json".to_string(),
            ),
            (
                GWT_BIN_PATH_ENV.to_string(),
                DOCKER_GWTD_BIN_PATH.to_string(),
            ),
        ]);

        finalize_docker_agent_launch_config(&project, &mut config).expect("finalize docker");

        assert_eq!(config.command, docker_binary_for_launch());
        assert!(config.args.windows(2).any(|pair| {
            pair[0] == "-f" && pair[1] == project.join("docker-compose.yml").display().to_string()
        }));
        assert!(config.args.contains(&"exec".to_string()));
        assert!(config.args.contains(&"app".to_string()));
        assert!(config.args.contains(&"codex".to_string()));
        assert!(config.args.contains(&"--no-alt-screen".to_string()));
    }

    #[test]
    fn resolve_launch_worktree_request_noops_when_repo_is_detached_and_base_is_missing() {
        let temp = tempdir().expect("tempdir");
        let repo = temp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo dir");

        let init = Command::new("git")
            .args(["init", "-q", "-b", "develop"])
            .current_dir(&repo)
            .status()
            .expect("git init");
        assert!(init.success(), "git init failed");
        let config_name = Command::new("git")
            .args(["config", "user.name", "Codex"])
            .current_dir(&repo)
            .status()
            .expect("git config user.name");
        assert!(config_name.success(), "git config user.name failed");
        let config_email = Command::new("git")
            .args(["config", "user.email", "codex@example.com"])
            .current_dir(&repo)
            .status()
            .expect("git config user.email");
        assert!(config_email.success(), "git config user.email failed");
        fs::write(repo.join("README.md"), "repo\n").expect("write readme");
        let add = Command::new("git")
            .args(["add", "README.md"])
            .current_dir(&repo)
            .status()
            .expect("git add");
        assert!(add.success(), "git add failed");
        let commit = Command::new("git")
            .args(["commit", "-qm", "init"])
            .current_dir(&repo)
            .status()
            .expect("git commit");
        assert!(commit.success(), "git commit failed");
        let detach = Command::new("git")
            .args(["checkout", "--detach"])
            .current_dir(&repo)
            .status()
            .expect("git checkout --detach");
        assert!(detach.success(), "git checkout --detach failed");

        let mut working_dir = None;
        let mut env_vars = HashMap::new();
        resolve_launch_worktree_request(
            &repo,
            Some("feature/demo"),
            None,
            &mut working_dir,
            &mut env_vars,
        )
        .expect("detached repo without base branch should no-op");

        assert!(working_dir.is_none());
        assert!(env_vars.is_empty());
    }
}
