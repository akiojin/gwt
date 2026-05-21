use super::*;

pub fn resolve_launch_worktree_request(
    repo_path: &Path,
    branch_name: Option<&str>,
    base_branch: &mut Option<String>,
    working_dir: &mut Option<PathBuf>,
    env_vars: &mut HashMap<String, String>,
) -> Result<(), String> {
    let Some(branch_name) = branch_name.map(str::to_string) else {
        return Ok(());
    };
    if working_dir.is_some() {
        return Ok(());
    }

    let main_repo_path = match gwt_git::worktree::main_worktree_root(repo_path) {
        Ok(path) => path,
        Err(error) => {
            if base_branch.is_none()
                && matches!(
                    gwt_git::detect_repo_type(repo_path),
                    gwt_git::RepoType::NonRepo
                )
            {
                return Ok(());
            }
            return Err(error.to_string());
        }
    };
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    let mut worktrees = manager.list().map_err(|err| err.to_string())?;
    if let Some(existing_worktree) = usable_worktree_path_for_branch(&worktrees, &branch_name) {
        *working_dir = Some(existing_worktree.clone());
        env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            existing_worktree.display().to_string(),
        );
        return Ok(());
    }
    if worktrees_have_stale_branch_entry(&worktrees, &branch_name) {
        manager
            .prune()
            .map_err(|err| format!("failed to prune stale worktrees: {err}"))?;
        worktrees = manager.list().map_err(|err| err.to_string())?;
        if let Some(existing_worktree) = usable_worktree_path_for_branch(&worktrees, &branch_name) {
            *working_dir = Some(existing_worktree.clone());
            env_vars.insert(
                "GWT_PROJECT_ROOT".to_string(),
                existing_worktree.display().to_string(),
            );
            return Ok(());
        }
    }

    let mut effective_base_branch = base_branch
        .clone()
        .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string());
    let mut remote_base_ref = origin_remote_ref(&effective_base_branch);
    let remote_branch_ref = origin_remote_ref(&branch_name);
    let has_local_branch = local_branch_exists(&main_repo_path, &branch_name)?;

    if !has_local_branch {
        if is_start_work_branch_name(&branch_name) {
            manager
                .prepare_start_work_remote_develop()
                .map_err(|err| format!("failed to prepare origin/develop for Start Work: {err}"))?;
            effective_base_branch = "origin/develop".to_string();
            remote_base_ref = origin_remote_ref(&effective_base_branch);
            *base_branch = Some(effective_base_branch.clone());
        } else {
            manager
                .fetch_origin()
                .map_err(|err| format!("failed to fetch origin: {err}"))?;
        }

        if !manager
            .remote_branch_exists(&remote_base_ref)
            .map_err(|err| {
                format!("failed to verify remote base branch {remote_base_ref}: {err}")
            })?
        {
            if let Some(fallback_base_branch) =
                gwt::start_work::refallback_start_work_base_branch_with(
                    &branch_name,
                    &effective_base_branch,
                    |candidate| {
                        let candidate_ref = origin_remote_ref(candidate);
                        manager.remote_branch_exists(&candidate_ref).map_err(|err| {
                            format!("failed to verify remote base branch {candidate_ref}: {err}")
                        })
                    },
                )?
            {
                effective_base_branch = fallback_base_branch;
                remote_base_ref = origin_remote_ref(&effective_base_branch);
                *base_branch = Some(effective_base_branch.clone());
            } else {
                return Err(format!(
                    "remote base branch does not exist: {remote_base_ref}"
                ));
            }
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

fn is_start_work_branch_name(branch_name: &str) -> bool {
    branch_name
        .strip_prefix("work/")
        .is_some_and(|name| !name.is_empty())
}

pub fn resolve_launch_worktree(
    repo_path: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    let mut base_branch = config.base_branch.clone();
    resolve_launch_worktree_request(
        repo_path,
        config.branch.as_deref(),
        &mut base_branch,
        &mut config.working_dir,
        &mut config.env_vars,
    )?;
    config.base_branch = base_branch;
    Ok(())
}

pub fn resolve_shell_launch_worktree(
    repo_path: &Path,
    config: &mut ShellLaunchConfig,
) -> Result<(), String> {
    let mut base_branch = config.base_branch.clone();
    resolve_launch_worktree_request(
        repo_path,
        config.branch.as_deref(),
        &mut base_branch,
        &mut config.working_dir,
        &mut config.env_vars,
    )?;
    config.base_branch = base_branch;
    Ok(())
}

pub fn build_shell_process_launch(
    repo_path: &Path,
    config: &mut ShellLaunchConfig,
) -> Result<ProcessLaunch, String> {
    let worktree = config
        .working_dir
        .clone()
        .unwrap_or_else(|| repo_path.to_path_buf());
    let base_env = if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
        gwt_agent::LaunchEnvironment::from_base_env(std::iter::empty::<(String, String)>())
    } else {
        gwt_agent::LaunchEnvironment::from_base_env(gwt_agent::environment::host_process_env())
    };
    let mut env = config.env_vars.clone();
    let mut remove_env = config.remove_env.clone();
    base_env.apply_to_parts(&mut env, &mut remove_env);

    if config.runtime_target != gwt_agent::LaunchRuntimeTarget::Docker {
        let windows_shell = if cfg!(windows) {
            config.windows_shell
        } else {
            None
        };
        let shell = match windows_shell {
            Some(windows_shell) => gwt::ShellProgram {
                command: windows_shell_process_command(windows_shell).to_string(),
                args: interactive_windows_shell_args(windows_shell),
            },
            None => detect_shell_program().map_err(|error| error.to_string())?,
        };
        env.insert(
            "GWT_PROJECT_ROOT".to_string(),
            worktree.display().to_string(),
        );
        install_launch_gwt_bin_env(&mut env, gwt_agent::LaunchRuntimeTarget::Host)?;
        config.env_vars = env.clone();
        return Ok(ProcessLaunch {
            command: shell.command,
            args: shell.args,
            env,
            remove_env,
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
        remove_env: Vec::new(),
        cwd: Some(worktree),
    })
}

pub const WINDOWS_HOST_SHELL_EXPRESSION_ENV: &str = "GWT_WINDOWS_HOST_SHELL_EXPRESSION";

pub fn windows_shell_process_command(shell: gwt_agent::WindowsShellKind) -> &'static str {
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

pub fn apply_windows_host_shell_wrapper(
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

pub fn apply_host_package_runner_fallback(config: &mut gwt_agent::LaunchConfig) -> bool {
    apply_host_package_runner_fallback_with_probe(
        config,
        "npx".to_string(),
        probe_host_package_runner,
    )
}

pub fn apply_host_package_runner_fallback_with_probe<F>(
    config: &mut gwt_agent::LaunchConfig,
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

fn resolve_host_package_runner_with_probe<F>(
    config: &gwt_agent::LaunchConfig,
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

fn host_package_runner_version_spec(config: &gwt_agent::LaunchConfig) -> Option<String> {
    package_runner_version_spec(config)
        .or_else(|| infer_package_runner_version_spec(&config.command, &config.args))
}

fn infer_package_runner_version_spec(command: &str, args: &[String]) -> Option<String> {
    if !(command_matches_runner(command, "bunx") || command_matches_runner(command, "npx")) {
        return None;
    }

    let version_spec = match args.first().map(String::as_str) {
        Some("--yes" | "-y") => args.get(1)?,
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
    probe_host_package_runner_with_timeout(
        command,
        args,
        env_vars,
        remove_env,
        cwd,
        Duration::from_secs(5),
        Duration::from_millis(50),
    )
}

pub fn probe_host_package_runner_with_timeout(
    command: &str,
    args: Vec<String>,
    env_vars: &HashMap<String, String>,
    remove_env: &[String],
    cwd: Option<PathBuf>,
    timeout: Duration,
    poll_interval: Duration,
) -> bool {
    // SPEC-1924 FR-039 / SPEC-2809 Phase D-agent — emit summary tracing
    // around the bounded-poll spawn so the Logs Process facet (kind =
    // agent) and the Console window see the launch attempt. stdio is
    // intentionally null because the caller only consumes the timeout +
    // exit status; nothing to forward to the hub.
    let agent_spawn_id =
        AGENT_LAUNCH_SPAWN_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let agent_label = format!("{} {}", command, args.join(" "));
    tracing::info!(
        target: "gwt.process.summary",
        kind = "agent",
        spawn_id = agent_spawn_id,
        label = %agent_label,
        phase = "start",
        "process start",
    );

    let mut process = gwt_core::process::hidden_command(command);
    process
        .args(&args)
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
    let Ok(mut child) = process.spawn() else {
        tracing::info!(
            target: "gwt.process.summary",
            kind = "agent",
            spawn_id = agent_spawn_id,
            label = %agent_label,
            phase = "end",
            exit_code = None::<i64>,
            success = false,
            error = "spawn failed",
            "process end",
        );
        return false;
    };
    let start = Instant::now();
    let emit_end = |exit_code: Option<i32>, success: bool, note: Option<&str>| {
        tracing::info!(
            target: "gwt.process.summary",
            kind = "agent",
            spawn_id = agent_spawn_id,
            label = %agent_label,
            phase = "end",
            exit_code = exit_code.map(|c| c as i64),
            duration_ms = start.elapsed().as_millis() as u64,
            success = success,
            note = note,
            "process end",
        );
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let success = status.success();
                emit_end(status.code(), success, None);
                return success;
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    emit_end(None, false, Some("timeout"));
                    return false;
                }
                thread::sleep(poll_interval);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                emit_end(None, false, Some("wait error"));
                return false;
            }
        }
    }
}

static AGENT_LAUNCH_SPAWN_COUNTER: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

pub fn command_matches_runner(command: &str, runner: &str) -> bool {
    let path = Path::new(command);
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .is_some_and(|name| name.eq_ignore_ascii_case(runner))
}

pub fn ensure_docker_launch_runtime_ready() -> Result<(), String> {
    let path = std::env::var("PATH").unwrap_or_default();
    let docker_bin = std::env::var("GWT_DOCKER_BIN").unwrap_or_else(|_| "docker".to_string());
    tracing::info!(
        target: "gwt::launch::preflight",
        runtime_target = "docker",
        attempted_binary = %docker_bin,
        path = %path,
        "docker preflight started"
    );
    let result = run_docker_preflight();
    match &result {
        Ok(()) => {
            tracing::info!(
                target: "gwt::launch::preflight",
                runtime_target = "docker",
                outcome = "ready",
                attempted_binary = %docker_bin,
                "docker preflight completed"
            );
        }
        Err(error) => {
            tracing::error!(
                target: "gwt::launch::preflight",
                runtime_target = "docker",
                outcome = "failed",
                attempted_binary = %docker_bin,
                path = %path,
                error = %error,
                "docker preflight failed"
            );
        }
    }
    result
}

fn run_docker_preflight() -> Result<(), String> {
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

pub fn install_launch_gwt_bin_env(
    env_vars: &mut HashMap<String, String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
) -> Result<(), String> {
    let current_exe = std::env::current_exe().map_err(|error| format!("current_exe: {error}"))?;
    install_launch_gwt_bin_env_with_lookup(env_vars, runtime_target, &current_exe, |command| {
        which::which(command).ok()
    })
}

pub fn install_launch_gwt_bin_env_with_lookup(
    env_vars: &mut HashMap<String, String>,
    runtime_target: gwt_agent::LaunchRuntimeTarget,
    current_exe: &Path,
    lookup: impl FnOnce(&str) -> Option<PathBuf>,
) -> Result<(), String> {
    let gwt_bin = match runtime_target {
        gwt_agent::LaunchRuntimeTarget::Docker => DOCKER_GWTD_BIN_PATH.to_string(),
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
    if let Some(resolved) = env_vars.get(gwt_agent::session::GWT_BIN_PATH_ENV).cloned() {
        if let Some(parent) = Path::new(&resolved).parent() {
            match runtime_target {
                gwt_agent::LaunchRuntimeTarget::Docker => {
                    gwt_agent::prepare::prepend_posix_dir_to_path(env_vars, parent);
                }
                gwt_agent::LaunchRuntimeTarget::Host => {
                    gwt_agent::prepare::prepend_dir_to_path(env_vars, parent);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_path(entries: &[&str]) -> String {
        std::env::join_paths(entries.iter().map(Path::new))
            .expect("join test PATH entries")
            .to_string_lossy()
            .into_owned()
    }

    fn posix_path_entries(path: &str) -> Vec<&str> {
        path.split(':').collect()
    }

    #[test]
    fn windows_shell_process_command_maps_all_variants() {
        assert_eq!(
            windows_shell_process_command(gwt_agent::WindowsShellKind::CommandPrompt),
            "cmd.exe"
        );
        assert_eq!(
            windows_shell_process_command(gwt_agent::WindowsShellKind::WindowsPowerShell),
            "powershell"
        );
        assert_eq!(
            windows_shell_process_command(gwt_agent::WindowsShellKind::PowerShell7),
            "pwsh"
        );
    }

    #[test]
    fn interactive_windows_shell_args_returns_expected_flags() {
        assert!(
            interactive_windows_shell_args(gwt_agent::WindowsShellKind::CommandPrompt).is_empty()
        );
        assert_eq!(
            interactive_windows_shell_args(gwt_agent::WindowsShellKind::WindowsPowerShell),
            vec!["-NoLogo"]
        );
        assert_eq!(
            interactive_windows_shell_args(gwt_agent::WindowsShellKind::PowerShell7),
            vec!["-NoLogo"]
        );
    }

    // SPEC-2077 Phase I1 (US-7 / FR-020 / FR-021 / FR-022 / SC-010):
    // launch_runtime mirror of install_launch_gwt_bin_env_with_lookup must
    // prepend dirname(GWT_BIN_PATH) to env_vars["PATH"] with dedup + empty
    // guard. Mirrors crates/gwt-agent/src/prepare.rs::tests::install_launch_gwt_bin_env_*.

    #[test]
    fn install_launch_gwt_bin_env_host_prepends_gwtd_dir_to_path() {
        let mut env_vars = HashMap::from([("PATH".to_string(), test_path(&["/usr/bin", "/bin"]))]);
        let current_exe = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwt");
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            gwt_agent::LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd")),
        )
        .expect("install");

        assert_eq!(
            env_vars
                .get(gwt_agent::session::GWT_BIN_PATH_ENV)
                .map(String::as_str),
            Some("/Applications/GWT.app/Contents/MacOS/gwtd"),
        );
        let entries: Vec<PathBuf> =
            std::env::split_paths(env_vars.get("PATH").expect("PATH")).collect();
        assert_eq!(
            entries.first().map(|p| p.as_path()),
            Some(Path::new("/Applications/GWT.app/Contents/MacOS")),
        );
        assert!(entries.contains(&PathBuf::from("/usr/bin")));
        assert!(entries.contains(&PathBuf::from("/bin")));
    }

    #[test]
    fn install_launch_gwt_bin_env_host_dedups_existing_path_entry() {
        let mut env_vars = HashMap::from([(
            "PATH".to_string(),
            test_path(&["/Applications/GWT.app/Contents/MacOS", "/usr/bin"]),
        )]);
        let current_exe = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwt");
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            gwt_agent::LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd")),
        )
        .expect("install");

        let entries: Vec<PathBuf> =
            std::env::split_paths(env_vars.get("PATH").expect("PATH")).collect();
        assert_eq!(
            entries,
            vec![
                PathBuf::from("/Applications/GWT.app/Contents/MacOS"),
                PathBuf::from("/usr/bin"),
            ],
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_host_skips_path_update_when_parent_is_empty() {
        let original_path = test_path(&["/usr/bin", "/bin"]);
        let mut env_vars = HashMap::from([("PATH".to_string(), original_path.clone())]);
        let current_exe = PathBuf::from("/opt/gwt/bin/gwt");
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            gwt_agent::LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(PathBuf::from("gwtd")),
        )
        .expect("install");

        // GWT_BIN_PATH may end up as a sibling/managed_assets resolution; we
        // assert only that PATH is untouched when the resolved binary has no
        // meaningful parent dir.
        assert_eq!(
            env_vars.get("PATH").map(String::as_str),
            Some(original_path.as_str()),
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_host_creates_path_when_absent() {
        let mut env_vars: HashMap<String, String> = HashMap::new();
        let current_exe = PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwt");
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            gwt_agent::LaunchRuntimeTarget::Host,
            &current_exe,
            |_command| Some(PathBuf::from("/Applications/GWT.app/Contents/MacOS/gwtd")),
        )
        .expect("install");

        let path = env_vars.get("PATH").expect("PATH should be created");
        let entries: Vec<PathBuf> = std::env::split_paths(path).collect();
        assert_eq!(
            entries,
            vec![PathBuf::from("/Applications/GWT.app/Contents/MacOS")],
        );
    }

    #[test]
    fn install_launch_gwt_bin_env_docker_prepends_when_dir_missing_from_path() {
        let mut env_vars = HashMap::from([("PATH".to_string(), "/usr/bin:/bin".to_string())]);
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            gwt_agent::LaunchRuntimeTarget::Docker,
            Path::new("/never/used/in/docker"),
            |_command| None,
        )
        .expect("install");

        assert_eq!(
            env_vars
                .get(gwt_agent::session::GWT_BIN_PATH_ENV)
                .map(String::as_str),
            Some("/usr/local/bin/gwtd"),
        );
        let entries = posix_path_entries(env_vars.get("PATH").expect("PATH"));
        assert_eq!(entries.first().copied(), Some("/usr/local/bin"),);
    }

    #[test]
    fn install_launch_gwt_bin_env_docker_dedups_when_dir_already_on_path() {
        let mut env_vars =
            HashMap::from([("PATH".to_string(), "/usr/local/bin:/usr/bin".to_string())]);
        install_launch_gwt_bin_env_with_lookup(
            &mut env_vars,
            gwt_agent::LaunchRuntimeTarget::Docker,
            Path::new("/never/used/in/docker"),
            |_command| None,
        )
        .expect("install");

        let entries = posix_path_entries(env_vars.get("PATH").expect("PATH"));
        assert_eq!(entries, vec!["/usr/local/bin", "/usr/bin"],);
    }
}
