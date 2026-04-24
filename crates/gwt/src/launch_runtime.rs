use super::*;

pub(crate) fn resolve_launch_worktree_request(
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
    if let Err(error) = &current_branch {
        if base_branch.is_none()
            && matches!(
                gwt_git::detect_repo_type(repo_path),
                gwt_git::RepoType::NonRepo
            )
        {
            return Ok(());
        }
        if base_branch.is_none() {
            return Err(error.clone());
        }
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
        .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string());
    let remote_base_ref = origin_remote_ref(&base_branch);
    let remote_branch_ref = origin_remote_ref(&branch_name);
    let has_local_branch = local_branch_exists(&main_repo_path, &branch_name)?;

    if !has_local_branch {
        manager
            .fetch_origin()
            .map_err(|err| format!("failed to fetch origin: {err}"))?;

        if !manager
            .remote_branch_exists(&remote_base_ref)
            .map_err(|err| {
                format!("failed to verify remote base branch {remote_base_ref}: {err}")
            })?
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
    }

    let preferred_worktree_path =
        gwt_git::worktree::sibling_worktree_path(&main_repo_path, &branch_name);
    let worktree_path = first_available_worktree_path(&preferred_worktree_path, &worktrees)
        .ok_or_else(|| {
            format!("failed to resolve available worktree path for branch {branch_name}")
        })?;
    if has_local_branch {
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

pub(crate) fn resolve_launch_worktree(
    repo_path: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    resolve_launch_worktree_request(
        repo_path,
        config.branch.as_deref(),
        config.base_branch.as_deref(),
        &mut config.working_dir,
        &mut config.env_vars,
    )
}

pub(crate) fn resolve_shell_launch_worktree(
    repo_path: &Path,
    config: &mut ShellLaunchConfig,
) -> Result<(), String> {
    resolve_launch_worktree_request(
        repo_path,
        config.branch.as_deref(),
        config.base_branch.as_deref(),
        &mut config.working_dir,
        &mut config.env_vars,
    )
}

pub(crate) fn build_shell_process_launch(
    repo_path: &Path,
    config: &mut ShellLaunchConfig,
) -> Result<ProcessLaunch, String> {
    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    let mut env = spawn_env();
    env.extend(config.env_vars.clone());

    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Docker {
        let shell = match config.windows_shell {
            Some(windows_shell) => gwt::ShellProgram {
                command: windows_shell_process_command(windows_shell).to_string(),
                args: interactive_windows_shell_args(windows_shell),
            },
            None => detect_shell_program().map_err(|error| error.to_string())?,
        };
        env.entry("GWT_PROJECT_ROOT".to_string())
            .or_insert_with(|| worktree.display().to_string());
        install_launch_gwt_bin_env(&mut env, gwt_agent::LaunchRuntimeTarget::Host)?;
        config.env_vars = env.clone();
        return Ok(ProcessLaunch {
            command: shell.command,
            args: shell.args,
            env,
            cwd: Some(worktree),
        });
    }

    let launch = resolve_docker_launch_plan(&worktree, config.docker_service.as_deref())?;
    ensure_docker_launch_runtime_ready()?;
    ensure_docker_gwt_binary_setup(&launch)?;
    ensure_docker_launch_service_ready(&launch, config.docker_lifecycle_intent)?;
    let shell_command = resolve_docker_shell_command(&launch)?;
    env.insert("GWT_PROJECT_ROOT".to_string(), launch.container_cwd.clone());
    install_launch_gwt_bin_env(&mut env, gwt_agent::LaunchRuntimeTarget::Docker)?;
    config.docker_service = Some(launch.service.clone());
    config.env_vars = env.clone();

    let mut args = vec![
        "compose".to_string(),
        "-f".to_string(),
        launch.compose_file.display().to_string(),
        "exec".to_string(),
        "-w".to_string(),
        launch.container_cwd.clone(),
    ];
    args.extend(docker_compose_exec_env_args(&env));
    args.push(launch.service);
    args.push(shell_command);

    Ok(ProcessLaunch {
        command: docker_binary_for_launch(),
        args,
        env,
        cwd: Some(worktree),
    })
}

pub(crate) const WINDOWS_HOST_SHELL_EXPRESSION_ENV: &str = "GWT_WINDOWS_HOST_SHELL_EXPRESSION";

pub(crate) fn windows_shell_process_command(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "cmd.exe",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "powershell",
        gwt_agent::WindowsShellKind::PowerShell7 => "pwsh",
    }
}

fn interactive_windows_shell_args(shell: gwt_agent::WindowsShellKind) -> Vec<String> {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => Vec::new(),
        gwt_agent::WindowsShellKind::WindowsPowerShell
        | gwt_agent::WindowsShellKind::PowerShell7 => vec!["-NoLogo".to_string()],
    }
}

pub(crate) fn apply_windows_host_shell_wrapper(
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Host {
        return Ok(());
    }
    let Some(shell) = config.windows_shell else {
        return Ok(());
    };

    let (command, args) =
        wrap_windows_host_shell_command(shell, &config.command, &config.args, &mut config.env_vars);
    config.command = command;
    config.args = args;
    Ok(())
}

fn wrap_windows_host_shell_command(
    shell: gwt_agent::WindowsShellKind,
    command: &str,
    args: &[String],
    env: &mut HashMap<String, String>,
) -> (String, Vec<String>) {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => {
            let expression = format!("{} & exit", build_cmd_command_expression(command, args));
            env.insert(WINDOWS_HOST_SHELL_EXPRESSION_ENV.to_string(), expression);
            (
                windows_shell_process_command(shell).to_string(),
                vec![
                    "/d".to_string(),
                    "/k".to_string(),
                    format!("%{WINDOWS_HOST_SHELL_EXPRESSION_ENV}%"),
                ],
            )
        }
        gwt_agent::WindowsShellKind::WindowsPowerShell
        | gwt_agent::WindowsShellKind::PowerShell7 => (
            windows_shell_process_command(shell).to_string(),
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-Command".to_string(),
                build_powershell_command_script(command, args),
            ],
        ),
    }
}

fn escape_cmd_double_quoted(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn quote_cmd_token_if_needed(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.chars().any(|c| {
            c.is_whitespace()
                || matches!(c, '&' | '|' | '<' | '>' | '(' | ')' | '^' | '%' | '!' | '"')
        });

    if needs_quotes {
        format!("\"{}\"", escape_cmd_double_quoted(value))
    } else {
        value.to_string()
    }
}

fn build_cmd_command_expression(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 2);
    parts.push("call".to_string());
    parts.push(quote_cmd_token_if_needed(command));
    parts.extend(args.iter().map(|arg| quote_cmd_token_if_needed(arg)));
    parts.join(" ")
}

fn quote_powershell_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn build_powershell_command_script(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_powershell_literal(command));
    parts.extend(args.iter().map(|arg| quote_powershell_literal(arg)));
    format!(
        "& {}; if ($null -ne $LASTEXITCODE) {{ exit $LASTEXITCODE }}; if (-not $?) {{ exit 1 }}",
        parts.join(" ")
    )
}

pub(crate) fn apply_host_package_runner_fallback(config: &mut gwt_agent::LaunchConfig) -> bool {
    apply_host_package_runner_fallback_with_probe(
        config,
        "npx".to_string(),
        probe_host_package_runner,
    )
}

pub(crate) fn apply_host_package_runner_fallback_with_probe<F>(
    config: &mut gwt_agent::LaunchConfig,
    fallback_executable: String,
    mut probe: F,
) -> bool
where
    F: FnMut(&str, Vec<String>, &HashMap<String, String>, Option<PathBuf>) -> bool,
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

fn resolve_host_package_runner_with_probe<F>(
    config: &gwt_agent::LaunchConfig,
    fallback_executable: String,
    probe: &mut F,
) -> Option<PackageRunnerProgram>
where
    F: FnMut(&str, Vec<String>, &HashMap<String, String>, Option<PathBuf>) -> bool,
{
    let version_spec = host_package_runner_version_spec(config)?;
    if !command_matches_runner(&config.command, "bunx") {
        return None;
    }

    let probe_args = vec![version_spec.clone(), "--version".to_string()];
    let cwd = config.working_dir.clone();
    if probe(&config.command, probe_args, &config.env_vars, cwd) {
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

fn host_package_runner_version_spec(config: &gwt_agent::LaunchConfig) -> Option<String> {
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
    cwd: Option<PathBuf>,
) -> bool {
    probe_host_package_runner_with_timeout(
        command,
        args,
        env_vars,
        cwd,
        Duration::from_secs(5),
        Duration::from_millis(50),
    )
}

pub(crate) fn probe_host_package_runner_with_timeout(
    command: &str,
    args: Vec<String>,
    env_vars: &HashMap<String, String>,
    cwd: Option<PathBuf>,
    timeout: Duration,
    poll_interval: Duration,
) -> bool {
    let mut process = Command::new(command);
    process
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .envs(env_vars);
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }
    let Ok(mut child) = process.spawn() else {
        return false;
    };
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return false;
                }
                thread::sleep(poll_interval);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

pub(crate) fn command_matches_runner(command: &str, runner: &str) -> bool {
    let path = Path::new(command);
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .is_some_and(|name| name.eq_ignore_ascii_case(runner))
}

pub(crate) fn ensure_docker_launch_runtime_ready() -> Result<(), String> {
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

pub(crate) fn install_launch_gwt_bin_env(
    env_vars: &mut HashMap<String, String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    install_launch_gwt_bin_env_with_lookup(env_vars, runtime_target, &current_exe, |command| {
        which::which(command).ok()
    })
}

pub(crate) fn install_launch_gwt_bin_env_with_lookup(
    env_vars: &mut HashMap<String, String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<(), String> {
    let gwt_bin = match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Docker => DOCKER_GWT_BIN_PATH.to_string(),
        gwt_agent::LaunchRuntimeTarget::Host => {
            gwt::managed_assets::resolve_public_gwt_bin_with_lookup(current_exe, lookup)
                .to_string_lossy()
                .into_owned()
        }
    };
    match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Docker => {
            env_vars.insert(gwt_agent::session::GWT_BIN_PATH_ENV.to_string(), gwt_bin);
        }
        gwt_agent::LaunchRuntimeTarget::Host => {
            env_vars
                .entry(gwt_agent::session::GWT_BIN_PATH_ENV.to_string())
                .or_insert(gwt_bin);
        }
    }
    Ok(())
}
