use super::*;

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::sync::{Arc, Mutex};

fn normalize_child_process_path(path: &Path) -> PathBuf {
    gwt_core::paths::normalize_windows_child_process_path(path)
}

fn normalize_launch_config_working_dir(config: &mut gwt_agent::LaunchConfig) {
    if let Some(dir) = config.working_dir.as_ref() {
        let normalized = normalize_child_process_path(dir);
        config.working_dir = Some(normalized.clone());
        config.env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            normalized.display().to_string(),
        );
    }
}

fn normalize_shell_launch_config_working_dir(config: &mut ShellLaunchConfig) {
    if let Some(dir) = config.working_dir.as_ref() {
        let normalized = normalize_child_process_path(dir);
        config.working_dir = Some(normalized.clone());
        config.env_vars.insert(
            "GWT_PROJECT_ROOT".to_string(),
            normalized.display().to_string(),
        );
    }
}

fn set_worktree_launch_path(
    working_dir: &mut Option<PathBuf>,
    env_vars: &mut HashMap<String, String>,
    path: &Path,
) {
    let path = normalize_child_process_path(path);
    *working_dir = Some(path.clone());
    env_vars.insert("GWT_PROJECT_ROOT".to_string(), path.display().to_string());
}

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
        set_worktree_launch_path(working_dir, env_vars, &existing_worktree);
        return Ok(());
    }
    if worktrees_have_stale_branch_entry(&worktrees, &branch_name) {
        manager
            .prune()
            .map_err(|err| format!("failed to prune stale worktrees: {err}"))?;
        worktrees = manager.list().map_err(|err| err.to_string())?;
        if let Some(existing_worktree) = usable_worktree_path_for_branch(&worktrees, &branch_name) {
            set_worktree_launch_path(working_dir, env_vars, &existing_worktree);
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

    set_worktree_launch_path(working_dir, env_vars, &worktree_path);
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
    normalize_launch_config_working_dir(config);
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
    normalize_shell_launch_config_working_dir(config);
    Ok(())
}

pub fn build_shell_process_launch(
    repo_path: &Path,
    config: &mut ShellLaunchConfig,
) -> Result<ProcessLaunch, String> {
    let worktree = normalize_child_process_path(
        &config
            .working_dir
            .clone()
            .unwrap_or_else(|| repo_path.to_path_buf()),
    );
    if config.working_dir.is_some() {
        config.working_dir = Some(worktree.clone());
    }
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

    let (normalized_command, normalized_args) =
        gwt_terminal::pty::normalize_command_for_windows_host_shell(
            &config.command,
            &config.args,
            &config.env_vars,
            &config.remove_env,
        );
    let (command, args) = wrap_windows_host_shell_command(
        shell,
        &normalized_command,
        &normalized_args,
        &mut config.env_vars,
    );
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
    let cwd = env.get("GWT_PROJECT_ROOT").map(String::as_str);
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => {
            let expression = build_cmd_wrapped_command_expression(command, args, cwd);
            env.insert(WINDOWS_HOST_SHELL_EXPRESSION_ENV.to_string(), expression);
            (
                windows_shell_process_command(shell).to_string(),
                vec![
                    "/d".to_string(),
                    "/v:on".to_string(),
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
                build_powershell_command_script(command, args, cwd),
            ],
        ),
    }
}

fn sensitive_launch_key(value: &str) -> bool {
    let normalized = value
        .trim_start_matches('-')
        .replace(['-', '_'], "")
        .to_ascii_lowercase();
    normalized == "apikey"
        || normalized == "token"
        || normalized == "authtoken"
        || normalized == "hooktoken"
        || normalized.ends_with("apikey")
        || normalized.ends_with("token")
        || normalized.contains("secret")
}

fn sanitize_launch_display_tokens(command: &str, args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len() + 1);
    out.push(command.to_string());
    let mut redact_next = false;
    for arg in args {
        if redact_next {
            out.push("[REDACTED]".to_string());
            redact_next = false;
            continue;
        }
        if let Some((key, _value)) = arg.split_once('=') {
            if sensitive_launch_key(key) {
                out.push(format!("{key}=[REDACTED]"));
                continue;
            }
        }
        if sensitive_launch_key(arg) {
            out.push(arg.clone());
            redact_next = true;
            continue;
        }
        out.push(arg.clone());
    }
    out
}

fn quote_display_token_if_needed(value: &str) -> String {
    if value.is_empty() || value.chars().any(char::is_whitespace) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_string()
    }
}

fn launch_display_command(command: &str, args: &[String]) -> String {
    let tokens = sanitize_launch_display_tokens(command, args);
    tokens
        .iter()
        .map(|token| quote_display_token_if_needed(&gwt_core::process_console::redact_line(token)))
        .collect::<Vec<_>>()
        .join(" ")
}

fn launch_banner_lines(command: &str, args: &[String], cwd: Option<&str>) -> Vec<String> {
    let mut lines = vec![
        "[gwt] launching agent".to_string(),
        "[gwt] runtime: host".to_string(),
    ];
    if let Some(cwd) = cwd.filter(|value| !value.is_empty()) {
        lines.push(format!("[gwt] cwd: {cwd}"));
    }
    lines.push(format!(
        "[gwt] command: {}",
        launch_display_command(command, args)
    ));
    lines
}

fn escape_cmd_echo_text(value: &str) -> String {
    value
        .replace('^', "^^")
        .replace('!', "^!")
        .replace('&', "^&")
        .replace('|', "^|")
        .replace('<', "^<")
        .replace('>', "^>")
        .replace('%', "^%")
}

fn build_cmd_wrapped_command_expression(
    command: &str,
    args: &[String],
    cwd: Option<&str>,
) -> String {
    let mut parts = launch_banner_lines(command, args, cwd)
        .into_iter()
        .map(|line| format!("echo {}", escape_cmd_echo_text(&line)))
        .collect::<Vec<_>>();
    parts.push(build_cmd_command_expression(command, args));
    parts.push("set GWT_AGENT_EXIT=!ERRORLEVEL!".to_string());
    parts.push("echo.".to_string());
    parts.push("echo [gwt] process exited with status !GWT_AGENT_EXIT!".to_string());
    parts.push("exit !GWT_AGENT_EXIT!".to_string());
    parts.join(" & ")
}

fn escape_cmd_double_quoted(value: &str) -> String {
    value.replace('!', "^!").replace('"', "\"\"")
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

/// Escape an argument for PowerShell's Legacy native command argument
/// passing so the child's MSVCRT command-line parser reconstructs the
/// original string (SPEC-2014 FR-105).
///
/// Legacy passing places the argument on the raw command line, wrapping it
/// in double quotes only when it contains whitespace and never escaping
/// embedded quotes. Unescaped `"` then toggle quoting in the child parser
/// and disappear: `{"fastMode":true}` arrives as `{fastMode:true}` and
/// Claude Code exits with `Error: Invalid JSON provided to --settings`.
/// Batch targets (`.cmd`/`.bat` such as npx.cmd) always use Legacy passing,
/// even under pwsh 7.3+'s `Windows` mode.
///
/// Only embedded quotes need help: a backslash run before each `"` is
/// doubled and the quote emitted as `\"`. Trailing backslashes are left
/// alone — Legacy passing already doubles a trailing run itself when it
/// wraps whitespace arguments (probe-verified on pwsh 7 and PS 5.1).
fn escape_native_arg_for_legacy_passing(value: &str) -> String {
    if !value.contains('"') {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len() + 8);
    let mut pending_backslashes = 0usize;
    for ch in value.chars() {
        match ch {
            '\\' => pending_backslashes += 1,
            '"' => {
                out.extend(std::iter::repeat_n('\\', pending_backslashes * 2 + 1));
                out.push('"');
                pending_backslashes = 0;
            }
            other => {
                out.extend(std::iter::repeat_n('\\', pending_backslashes));
                pending_backslashes = 0;
                out.push(other);
            }
        }
    }
    out.extend(std::iter::repeat_n('\\', pending_backslashes));
    out
}

fn build_powershell_command_script(command: &str, args: &[String], cwd: Option<&str>) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_powershell_literal(command));
    parts.extend(
        args.iter()
            .map(|arg| quote_powershell_literal(&escape_native_arg_for_legacy_passing(arg))),
    );
    // Pin Legacy passing so the escaping above is deterministic across
    // PowerShell versions and target kinds (.exe vs .cmd). Windows
    // PowerShell 5.1 ignores the assignment and is Legacy-only anyway.
    let mut script = vec!["$PSNativeCommandArgumentPassing = 'Legacy'".to_string()];
    script.extend(
        launch_banner_lines(command, args, cwd)
            .into_iter()
            .map(|line| format!("Write-Host {}", quote_powershell_literal(&line))),
    );
    script.push(format!("& {}", parts.join(" ")));
    script.push(
        "$gwtExitCode = if ($null -ne $LASTEXITCODE) { $LASTEXITCODE } elseif ($?) { 0 } else { 1 }"
            .to_string(),
    );
    script.push("Write-Host ''".to_string());
    script.push("Write-Host \"[gwt] process exited with status $gwtExitCode\"".to_string());
    script.push("exit $gwtExitCode".to_string());
    script.join("; ")
}

#[cfg(test)]
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostPackageRunnerFallbackReport {
    pub switched_to_fallback: bool,
    pub repaired_npx_cache: bool,
    pub messages: Vec<String>,
}

pub fn apply_host_package_runner_fallback_checked(
    config: &mut gwt_agent::LaunchConfig,
) -> Result<HostPackageRunnerFallbackReport, String> {
    // Issue #2981: resolve a Windows-spawnable npx (prefers `npx.cmd`) instead of
    // a bare `npx` that `CreateProcess` cannot launch after a failed bunx probe.
    let fallback_executable = gwt_agent::resolve_host_npx_fallback_executable(&config.env_vars);
    apply_host_package_runner_fallback_checked_with_probe_and_repair(
        config,
        fallback_executable,
        default_windows_npx_cache_base(),
        probe_host_package_runner_outcome,
        repair_windows_npx_cache,
    )
}

fn apply_host_package_runner_fallback_checked_with_probe_and_repair<F, R>(
    config: &mut gwt_agent::LaunchConfig,
    fallback_executable: String,
    npx_cache_base: Option<PathBuf>,
    mut probe: F,
    mut repair: R,
) -> Result<HostPackageRunnerFallbackReport, String>
where
    F: FnMut(
        &str,
        Vec<String>,
        &HashMap<String, String>,
        &[String],
        Option<PathBuf>,
    ) -> PackageRunnerProbeOutcome,
    R: FnMut(&WindowsNpxCacheRepairCandidate) -> Result<(), String>,
{
    let Some(version_spec) = host_package_runner_version_spec(config) else {
        return Ok(HostPackageRunnerFallbackReport::default());
    };
    if !command_matches_runner(&config.command, "bunx") {
        return Ok(HostPackageRunnerFallbackReport::default());
    }

    let cwd = config.working_dir.clone();
    let bunx_probe = probe(
        &config.command,
        vec![version_spec.clone(), "--version".to_string()],
        &config.env_vars,
        &config.remove_env,
        cwd.clone(),
    );
    if bunx_probe.success {
        return Ok(HostPackageRunnerFallbackReport::default());
    }

    let agent_args = strip_package_runner_args(&config.args, &version_spec);
    let mut fallback_args = vec!["--yes".to_string(), version_spec.clone()];
    fallback_args.extend(agent_args);
    let fallback_probe_args = vec![
        "--yes".to_string(),
        version_spec.clone(),
        "--version".to_string(),
    ];
    let mut report = HostPackageRunnerFallbackReport::default();
    let first_npx_probe = probe(
        &fallback_executable,
        fallback_probe_args.clone(),
        &config.env_vars,
        &config.remove_env,
        cwd.clone(),
    );
    if first_npx_probe.success {
        config.command = fallback_executable;
        config.args = fallback_args;
        report.switched_to_fallback = true;
        report
            .messages
            .push("bunx unavailable, switching to npx...".to_string());
        return Ok(report);
    }

    let probe_output = first_npx_probe.combined_output();
    let repair_candidate = npx_cache_base
        .as_deref()
        .and_then(|base| detect_windows_npx_cache_corruption(&probe_output, base));
    let Some(repair_candidate) = repair_candidate else {
        return Err(format!(
            "npx package-runner probe failed for {version_spec}. {} Manual recovery: run `npx --yes {version_spec} --version` in a terminal and repair the reported npm `_npx` directory if npm reports a missing executable.",
            first_npx_probe.diagnostic()
        ));
    };

    report.repaired_npx_cache = true;
    report.messages.push(format!(
        "Detected broken npm npx cache; repairing {}...",
        repair_candidate.npx_root.display()
    ));
    repair(&repair_candidate).map_err(|error| {
        format!(
            "Failed to repair npm npx cache at {}: {error}. Manual recovery: remove this `_npx` directory and retry the launch.",
            repair_candidate.npx_root.display()
        )
    })?;
    report
        .messages
        .push("npm npx cache repair succeeded; retrying launch...".to_string());

    let second_npx_probe = probe(
        &fallback_executable,
        fallback_probe_args,
        &config.env_vars,
        &config.remove_env,
        cwd,
    );
    if !second_npx_probe.success {
        return Err(format!(
            "npx package-runner probe failed after repairing npm npx cache at {}. {} Manual recovery: remove this `_npx` directory and retry the launch.",
            repair_candidate.npx_root.display(),
            second_npx_probe.diagnostic()
        ));
    }

    config.command = fallback_executable;
    config.args = fallback_args;
    report.switched_to_fallback = true;
    report
        .messages
        .push("bunx unavailable, switching to npx...".to_string());
    Ok(report)
}

#[cfg(test)]
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

fn probe_host_package_runner_outcome(
    command: &str,
    args: Vec<String>,
    env_vars: &HashMap<String, String>,
    remove_env: &[String],
    cwd: Option<PathBuf>,
) -> PackageRunnerProbeOutcome {
    #[cfg(windows)]
    {
        // Windows keeps executing the target package so the corrupt npm `_npx`
        // cache auto-repair (`895ccadce`) can inspect the failed runner output.
        let hub = gwt_core::process_console::global();
        probe_host_package_runner_with_timeout_and_hub(
            PackageRunnerProbeRequest {
                command,
                args,
                env_vars,
                remove_env,
                cwd,
                timeout: Duration::from_secs(5),
                poll_interval: Duration::from_millis(50),
            },
            &hub,
        )
    }
    #[cfg(not(windows))]
    {
        // Non-Windows only verifies that the runner *binary* resolves. Running
        // `<runner> <package> --version` to "validate" the runner downloads the
        // whole package; on a cold/slow first run that exceeds any probe budget
        // and aborts the launch with a misleading error card (issue #2948).
        // A binary-availability check is instant and never blocks, so the real
        // package download and any genuine runner error surface in the agent's
        // raw TTY instead of a preparation-error card.
        let _ = (args, remove_env, cwd);
        host_package_runner_binary_outcome(command, env_vars)
    }
}

/// Build a probe outcome from runner *binary* availability without executing
/// the target package. `success` means the runner resolves on PATH; it never
/// reports `timed_out`, so a slow package download can no longer fail the probe.
#[cfg(not(windows))]
fn host_package_runner_binary_outcome(
    command: &str,
    env_vars: &HashMap<String, String>,
) -> PackageRunnerProbeOutcome {
    let available = runner_binary_available(command, env_vars);
    PackageRunnerProbeOutcome {
        success: available,
        exit_code: Some(if available { 0 } else { 127 }),
        stdout: String::new(),
        stderr: String::new(),
        timed_out: false,
        error: None,
    }
}

/// Resolve whether a package-runner binary exists in the launch environment.
/// Absolute paths are trusted by existence; bare names are resolved against the
/// launch env `PATH` (mirroring the PTY spawn) so the decision matches what the
/// real launch will execute.
#[cfg(not(windows))]
fn runner_binary_available(command: &str, env_vars: &HashMap<String, String>) -> bool {
    let candidate = Path::new(command);
    if candidate.is_absolute() {
        return candidate.exists();
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    match env_vars.get("PATH") {
        Some(path) => which::which_in(command, Some(path.as_str()), &cwd).is_ok(),
        None => which::which(command).is_ok(),
    }
}

#[cfg(test)]
pub fn probe_host_package_runner_with_timeout(
    command: &str,
    args: Vec<String>,
    env_vars: &HashMap<String, String>,
    remove_env: &[String],
    cwd: Option<PathBuf>,
    timeout: Duration,
    poll_interval: Duration,
) -> bool {
    let hub = gwt_core::process_console::global();
    probe_host_package_runner_with_timeout_and_hub(
        PackageRunnerProbeRequest {
            command,
            args,
            env_vars,
            remove_env,
            cwd,
            timeout,
            poll_interval,
        },
        &hub,
    )
    .success
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PackageRunnerProbeOutcome {
    success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
    timed_out: bool,
    error: Option<String>,
}

impl PackageRunnerProbeOutcome {
    #[cfg(all(test, windows))]
    fn success() -> Self {
        Self {
            success: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            error: None,
        }
    }

    #[cfg(all(test, windows))]
    fn failure_with_stderr(stderr: &str) -> Self {
        Self {
            success: false,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: stderr.to_string(),
            timed_out: false,
            error: None,
        }
    }

    fn combined_output(&self) -> String {
        format!("{}\n{}", self.stdout, self.stderr)
    }

    fn diagnostic(&self) -> String {
        let mut parts = Vec::new();
        if let Some(error) = &self.error {
            parts.push(error.clone());
        }
        if self.timed_out {
            parts.push("probe timed out".to_string());
        }
        if let Some(code) = self.exit_code {
            parts.push(format!("exit status {code}"));
        }
        let output = self.combined_output();
        let output = output.trim();
        if !output.is_empty() {
            let redacted = gwt_core::process_console::redact_line(output);
            parts.push(truncate_diagnostic(&redacted, 1200));
        }
        if parts.is_empty() {
            "probe failed without output.".to_string()
        } else {
            format!("Probe detail: {}.", parts.join("; "))
        }
    }
}

fn truncate_diagnostic(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let truncated: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

// Only Windows still executes the package runner during launch (for `_npx`
// cache repair); other platforms resolve by binary availability (#2948), so the
// bounded-poll probe machinery below is unused on non-Windows non-test builds.
#[allow(dead_code)]
struct PackageRunnerProbeRequest<'a> {
    command: &'a str,
    args: Vec<String>,
    env_vars: &'a HashMap<String, String>,
    remove_env: &'a [String],
    cwd: Option<PathBuf>,
    timeout: Duration,
    poll_interval: Duration,
}

#[allow(dead_code)]
fn probe_host_package_runner_with_timeout_and_hub(
    request: PackageRunnerProbeRequest<'_>,
    hub: &gwt_core::process_console::ProcessConsoleHub,
) -> PackageRunnerProbeOutcome {
    let PackageRunnerProbeRequest {
        command,
        args,
        env_vars,
        remove_env,
        cwd,
        timeout,
        poll_interval,
    } = request;
    // SPEC-1924 FR-039 / SPEC-2809 Phase D-agent — emit summary tracing
    // around the bounded-poll spawn and forward probe stdout/stderr into
    // the ProcessConsoleHub so failed package-runner probes are inspectable.
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
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for key in remove_env {
        process.env_remove(key);
    }
    process.envs(env_vars);
    if let Some(cwd) = cwd {
        process.current_dir(cwd);
    }
    let mut child = match process.spawn() {
        Ok(child) => child,
        Err(error) => {
            let message = format!("[gwt] failed to start package-runner probe: {error}");
            push_probe_console_line(
                hub,
                agent_spawn_id,
                gwt_core::process_console::ProcessStream::Stderr,
                &message,
            );
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
            return PackageRunnerProbeOutcome {
                success: false,
                exit_code: None,
                stdout: String::new(),
                stderr: message,
                timed_out: false,
                error: Some(error.to_string()),
            };
        }
    };
    let captured_stdout = Arc::new(Mutex::new(String::new()));
    let captured_stderr = Arc::new(Mutex::new(String::new()));
    let stdout_forwarder = child.stdout.take().map(|stdout| {
        forward_probe_stream(
            stdout,
            hub.clone(),
            agent_spawn_id,
            gwt_core::process_console::ProcessStream::Stdout,
            Arc::clone(&captured_stdout),
        )
    });
    let stderr_forwarder = child.stderr.take().map(|stderr| {
        forward_probe_stream(
            stderr,
            hub.clone(),
            agent_spawn_id,
            gwt_core::process_console::ProcessStream::Stderr,
            Arc::clone(&captured_stderr),
        )
    });
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
    let join_forwarders = |stdout_forwarder: Option<JoinHandle<()>>,
                           stderr_forwarder: Option<JoinHandle<()>>| {
        if let Some(handle) = stdout_forwarder {
            let _ = handle.join();
        }
        if let Some(handle) = stderr_forwarder {
            let _ = handle.join();
        }
    };
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                join_forwarders(stdout_forwarder, stderr_forwarder);
                let success = status.success();
                emit_end(status.code(), success, None);
                return PackageRunnerProbeOutcome {
                    success,
                    exit_code: status.code(),
                    stdout: captured_string(&captured_stdout),
                    stderr: captured_string(&captured_stderr),
                    timed_out: false,
                    error: None,
                };
            }
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    emit_end(None, false, Some("timeout"));
                    return PackageRunnerProbeOutcome {
                        success: false,
                        exit_code: None,
                        stdout: captured_string(&captured_stdout),
                        stderr: captured_string(&captured_stderr),
                        timed_out: true,
                        error: None,
                    };
                }
                thread::sleep(poll_interval);
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                emit_end(None, false, Some("wait error"));
                return PackageRunnerProbeOutcome {
                    success: false,
                    exit_code: None,
                    stdout: captured_string(&captured_stdout),
                    stderr: captured_string(&captured_stderr),
                    timed_out: false,
                    error: Some("wait error".to_string()),
                };
            }
        }
    }
}

#[allow(dead_code)]
fn forward_probe_stream<R>(
    reader: R,
    hub: gwt_core::process_console::ProcessConsoleHub,
    spawn_id: u64,
    stream: gwt_core::process_console::ProcessStream,
    captured: Arc<Mutex<String>>,
) -> JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    for piece in line.trim_end_matches(['\r', '\n']).split('\r') {
                        if let Ok(mut captured) = captured.lock() {
                            captured.push_str(piece);
                            captured.push('\n');
                        }
                        push_probe_console_line(&hub, spawn_id, stream, piece);
                    }
                }
                Err(_) => break,
            }
        }
    })
}

#[allow(dead_code)]
fn captured_string(captured: &Arc<Mutex<String>>) -> String {
    captured
        .lock()
        .map(|value| value.clone())
        .unwrap_or_else(|_| String::new())
}

#[allow(dead_code)]
fn push_probe_console_line(
    hub: &gwt_core::process_console::ProcessConsoleHub,
    spawn_id: u64,
    stream: gwt_core::process_console::ProcessStream,
    message: &str,
) {
    if message.is_empty() {
        return;
    }
    let stripped = gwt_core::process_console::strip_ansi(message);
    let redacted = gwt_core::process_console::redact_line(&stripped);
    hub.push(gwt_core::process_console::ProcessLine::new(
        gwt_core::process_console::ProcessKind::AgentBootstrap,
        spawn_id,
        stream,
        redacted,
    ));
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WindowsNpxCacheRepairCandidate {
    npx_root: PathBuf,
    missing_binary: PathBuf,
}

fn detect_windows_npx_cache_corruption(
    output: &str,
    npx_cache_base: &Path,
) -> Option<WindowsNpxCacheRepairCandidate> {
    #[cfg(not(windows))]
    {
        let _ = output;
        let _ = npx_cache_base;
        None
    }
    #[cfg(windows)]
    {
        let npx_cache_base = lexical_normalize_path(npx_cache_base);
        for candidate in extract_windows_exe_paths(output) {
            let missing_binary = lexical_normalize_path(Path::new(&candidate));
            if !missing_binary.starts_with(&npx_cache_base) || missing_binary.exists() {
                continue;
            }
            let relative = missing_binary.strip_prefix(&npx_cache_base).ok()?;
            let mut components = relative.components();
            let hash = components.next()?.as_os_str();
            if hash.is_empty() {
                continue;
            }
            let npx_root = npx_cache_base.join(hash);
            if !npx_root.is_dir() || !has_old_binary_marker(&missing_binary) {
                continue;
            }
            return Some(WindowsNpxCacheRepairCandidate {
                npx_root,
                missing_binary,
            });
        }
        None
    }
}

#[cfg(windows)]
fn extract_windows_exe_paths(output: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for segment in output.split(['"', '\'']) {
        collect_windows_exe_path_candidate(segment, &mut paths);
    }
    for token in output.split_whitespace() {
        collect_windows_exe_path_candidate(token, &mut paths);
    }
    paths.sort();
    paths.dedup();
    paths
}

#[cfg(windows)]
fn collect_windows_exe_path_candidate(segment: &str, paths: &mut Vec<String>) {
    let normalized = segment
        .trim_matches(|ch: char| ch == '`' || ch == ',' || ch == ';')
        .replace('/', "\\");
    let lower = normalized.to_ascii_lowercase();
    let Some(start) = lower.find("\\npm-cache\\_npx\\") else {
        return;
    };
    let Some(exe_end) = lower[start..].find(".exe").map(|index| start + index + 4) else {
        return;
    };
    let prefix_start = find_windows_path_start(&normalized, start).unwrap_or_else(|| {
        normalized[..start]
            .rfind(char::is_whitespace)
            .map_or(0, |i| i + 1)
    });
    let mut candidate = normalized[prefix_start..exe_end].to_string();
    while candidate.contains("\\\\") {
        candidate = candidate.replace("\\\\", "\\");
    }
    if !candidate.is_empty() {
        paths.push(candidate);
    }
}

#[cfg(windows)]
fn find_windows_path_start(value: &str, end: usize) -> Option<usize> {
    let bytes = value.as_bytes();
    let max = end.saturating_sub(2).min(bytes.len().saturating_sub(2));
    (0..=max).rev().find(|&index| {
        bytes[index].is_ascii_alphabetic()
            && bytes.get(index + 1) == Some(&b':')
            && bytes
                .get(index + 2)
                .is_some_and(|separator| *separator == b'\\' || *separator == b'/')
    })
}

#[cfg(windows)]
fn lexical_normalize_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            _ => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

#[cfg(windows)]
fn has_old_binary_marker(missing_binary: &Path) -> bool {
    let Some(parent) = missing_binary.parent() else {
        return false;
    };
    let Some(file_name) = missing_binary.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let prefix = format!("{file_name}.old.");
    let Ok(entries) = fs::read_dir(parent) else {
        return false;
    };
    entries.flatten().any(|entry| {
        entry
            .file_name()
            .to_str()
            .and_then(|name| name.strip_prefix(&prefix))
            .is_some_and(|suffix| {
                !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit())
            })
    })
}

fn default_windows_npx_cache_base() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA")
            .map(|base| PathBuf::from(base).join("npm-cache").join("_npx"))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

fn repair_windows_npx_cache(candidate: &WindowsNpxCacheRepairCandidate) -> Result<(), String> {
    fs::remove_dir_all(&candidate.npx_root).map_err(|error| error.to_string())
}

#[allow(dead_code)]
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
    gwt_docker::launch_preflight()
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
    use tempfile::tempdir;

    fn test_path(entries: &[&str]) -> String {
        std::env::join_paths(entries.iter().map(Path::new))
            .expect("join test PATH entries")
            .to_string_lossy()
            .into_owned()
    }

    fn posix_path_entries(path: &str) -> Vec<&str> {
        path.split(':').collect()
    }

    fn sample_versioned_launch_config() -> gwt_agent::LaunchConfig {
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::ClaudeCode)
            .working_dir("E:/gwt/develop")
            .version("latest")
            .build();
        config.command = "bunx".to_string();
        config.args = vec![
            "@anthropic-ai/claude-code@latest".to_string(),
            "--print".to_string(),
        ];
        config.env_vars = HashMap::from([("TERM".to_string(), "xterm-256color".to_string())]);
        config.working_dir = Some(PathBuf::from("E:/gwt/develop"));
        config.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
        config.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Connect;
        config
    }

    fn run_git(repo: &Path, args: &[&str]) {
        let output = gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }

    fn git_status(repo: &Path, args: &[&str]) -> bool {
        gwt_core::process::hidden_command("git")
            .args(args)
            .current_dir(repo)
            .status()
            .expect("git status")
            .success()
    }

    #[test]
    fn start_work_launch_materialization_prepares_origin_develop_at_launch_time() {
        let temp = tempdir().expect("tempdir");
        let origin = temp.path().join("origin.git");
        let repo = temp.path().join("repo");
        run_git(temp.path(), &["init", "--bare", origin.to_str().unwrap()]);
        run_git(
            temp.path(),
            &["clone", origin.to_str().unwrap(), repo.to_str().unwrap()],
        );
        run_git(&repo, &["config", "user.email", "gwt@example.invalid"]);
        run_git(&repo, &["config", "user.name", "gwt"]);
        run_git(&repo, &["checkout", "-qb", "develop"]);
        fs::write(repo.join("README.md"), "develop\n").expect("write readme");
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "-m", "seed develop"]);
        run_git(&repo, &["push", "-u", "origin", "develop"]);
        run_git(&origin, &["symbolic-ref", "HEAD", "refs/heads/develop"]);
        run_git(&repo, &["remote", "set-head", "origin", "-a"]);
        run_git(&repo, &["checkout", "-qb", "main"]);
        fs::write(repo.join("README.md"), "main\n").expect("write readme");
        run_git(&repo, &["commit", "-am", "seed main"]);
        run_git(&repo, &["push", "-u", "origin", "main"]);
        run_git(&origin, &["symbolic-ref", "HEAD", "refs/heads/main"]);
        run_git(&repo, &["remote", "set-head", "origin", "-a"]);
        run_git(&repo, &["checkout", "develop"]);
        run_git(&origin, &["branch", "-D", "develop"]);
        run_git(&repo, &["update-ref", "-d", "refs/remotes/origin/develop"]);
        assert!(
            !git_status(
                &repo,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    "refs/remotes/origin/develop"
                ],
            ),
            "fixture should start without local origin/develop"
        );

        let mut base_branch = Some("origin/develop".to_string());
        let mut working_dir = None;
        let mut env_vars = HashMap::new();

        resolve_launch_worktree_request(
            &repo,
            Some("work/20260607-1200"),
            &mut base_branch,
            &mut working_dir,
            &mut env_vars,
        )
        .expect("resolve Start Work launch worktree");

        assert_eq!(base_branch.as_deref(), Some("origin/develop"));
        assert!(
            git_status(
                &repo,
                &[
                    "show-ref",
                    "--verify",
                    "--quiet",
                    "refs/remotes/origin/develop"
                ],
            ),
            "final Start Work launch must prepare origin/develop"
        );
        let worktree = working_dir.expect("launch worktree path");
        assert!(
            worktree.exists(),
            "final Start Work launch must materialize a worktree"
        );
        assert_eq!(
            env_vars.get("GWT_PROJECT_ROOT").map(String::as_str),
            Some(worktree.to_str().expect("utf8 worktree")),
        );
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

    #[test]
    fn powershell_agent_wrapper_prints_terminal_launch_banner_and_exit_status() {
        let mut env = HashMap::from([(
            "GWT_PROJECT_ROOT".to_string(),
            r"E:\gwt\work\demo".to_string(),
        )]);
        let args = vec![
            "--yes".to_string(),
            "@anthropic-ai/claude-code@latest".to_string(),
            "--api-key".to_string(),
            "raw-secret-value".to_string(),
        ];

        let (command, shell_args) = wrap_windows_host_shell_command(
            gwt_agent::WindowsShellKind::PowerShell7,
            r"C:\Program Files\nodejs\npx.cmd",
            &args,
            &mut env,
        );

        assert_eq!(command, "pwsh");
        let script = shell_args.last().expect("PowerShell command script");
        assert!(script.contains("[gwt] launching agent"));
        assert!(script.contains("[gwt] runtime: host"));
        assert!(script.contains(r"[gwt] cwd: E:\gwt\work\demo"));
        assert!(script.contains("[gwt] command:"));
        assert!(script.contains("[REDACTED]"));
        let display_line = script
            .split(';')
            .find(|line| line.contains("[gwt] command:"))
            .expect("display command banner line");
        assert!(!display_line.contains("raw-secret-value"));
        assert!(script.contains("[gwt] process exited with status"));
        assert!(script.contains("exit $gwtExitCode"));
    }

    #[test]
    fn command_prompt_agent_wrapper_prints_terminal_launch_banner_and_exit_status() {
        let mut env = HashMap::from([(
            "GWT_PROJECT_ROOT".to_string(),
            r"E:\gwt\work\demo".to_string(),
        )]);
        let args = vec![
            "--yes".to_string(),
            "@anthropic-ai/claude-code@latest".to_string(),
        ];

        let (command, shell_args) = wrap_windows_host_shell_command(
            gwt_agent::WindowsShellKind::CommandPrompt,
            "npx.cmd",
            &args,
            &mut env,
        );

        assert_eq!(command, "cmd.exe");
        assert_eq!(
            shell_args,
            vec![
                "/d".to_string(),
                "/v:on".to_string(),
                "/k".to_string(),
                format!("%{WINDOWS_HOST_SHELL_EXPRESSION_ENV}%")
            ],
        );
        let expression = env
            .get(WINDOWS_HOST_SHELL_EXPRESSION_ENV)
            .expect("cmd shell expression");
        assert!(expression.contains("[gwt] launching agent"));
        assert!(expression.contains("[gwt] runtime: host"));
        assert!(expression.contains(r"[gwt] cwd: E:\gwt\work\demo"));
        assert!(expression.contains("[gwt] command:"));
        assert!(expression.contains("[gwt] process exited with status !GWT_AGENT_EXIT!"));
        assert!(expression.contains("exit !GWT_AGENT_EXIT!"));
    }

    // SPEC-2014 FR-105 / SC-063: PowerShell wrappers must deliver arguments
    // containing embedded quotes intact. Legacy native passing places the
    // argument raw on the child command line, so embedded `"` must be
    // MSVCRT-escaped or the child argv loses them
    // (`{"fastMode":true}` arrives as `{fastMode:true}`).
    #[test]
    fn powershell_agent_wrapper_forces_legacy_passing_and_escapes_quoted_args() {
        let mut env = HashMap::from([(
            "GWT_PROJECT_ROOT".to_string(),
            r"E:\gwt\work\demo".to_string(),
        )]);
        let args = vec![
            "--yes".to_string(),
            "@anthropic-ai/claude-code@latest".to_string(),
            "--settings".to_string(),
            r#"{"fastMode":true}"#.to_string(),
        ];

        let (command, shell_args) = wrap_windows_host_shell_command(
            gwt_agent::WindowsShellKind::PowerShell7,
            r"C:\Program Files\nodejs\npx.cmd",
            &args,
            &mut env,
        );

        assert_eq!(command, "pwsh");
        let script = shell_args.last().expect("PowerShell command script");
        assert!(
            script.starts_with("$PSNativeCommandArgumentPassing = 'Legacy'"),
            "script must pin Legacy native argument passing first: {script}"
        );
        assert!(
            script.contains(r#"'{\"fastMode\":true}'"#),
            "native invocation must carry MSVCRT-escaped JSON: {script}"
        );
        let native_invocation = script
            .split("; ")
            .find(|stmt| stmt.trim_start().starts_with("& "))
            .expect("native invocation statement");
        assert!(
            !native_invocation.contains(r#"{"fastMode":true}"#),
            "unescaped JSON must not reach the native invocation: {native_invocation}"
        );
    }

    #[test]
    fn escape_native_arg_for_legacy_passing_rules() {
        // Arguments without embedded quotes (and without a wrapped trailing
        // backslash) pass through unchanged.
        assert_eq!(escape_native_arg_for_legacy_passing("--yes"), "--yes");
        assert_eq!(
            escape_native_arg_for_legacy_passing(r"E:\gwt\work\demo"),
            r"E:\gwt\work\demo"
        );
        // Embedded quotes are MSVCRT-escaped.
        assert_eq!(
            escape_native_arg_for_legacy_passing(r#"{"fastMode":true}"#),
            r#"{\"fastMode\":true}"#
        );
        // Backslashes immediately before a quote are doubled.
        assert_eq!(escape_native_arg_for_legacy_passing(r#"a\"b"#), r#"a\\\"b"#);
        // Trailing backslashes stay untouched: Legacy passing doubles a
        // trailing run itself when it wraps whitespace arguments.
        assert_eq!(
            escape_native_arg_for_legacy_passing("E:\\path with space\\"),
            "E:\\path with space\\"
        );
        // Whitespace and embedded quotes combined.
        assert_eq!(
            escape_native_arg_for_legacy_passing(r#"{"a":"hello world"}"#),
            r#"{\"a\":\"hello world\"}"#
        );
    }

    #[cfg(windows)]
    #[test]
    fn command_prompt_agent_wrapper_normalizes_bun_claude_stub_before_shell_expression() {
        let temp = tempdir().expect("tempdir");
        let bun_bin_dir = temp.path().join(".bun").join("bin");
        fs::create_dir_all(&bun_bin_dir).expect("bun bin");
        let global_shim = bun_bin_dir.join("claude.exe");
        fs::write(&global_shim, b"MZ\x00\x00bun-global-shim").expect("global shim");

        let package_root = temp
            .path()
            .join(".bun")
            .join("install")
            .join("global")
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        let package_bin_dir = package_root.join("bin");
        fs::create_dir_all(&package_bin_dir).expect("package bin");
        let placeholder_stub = package_bin_dir.join("claude.exe");
        fs::write(
            &placeholder_stub,
            "echo \"Error: claude native binary not installed.\" >&2\nexit 1\n",
        )
        .expect("placeholder stub");
        let cli_wrapper = package_root.join("cli-wrapper.cjs");
        fs::write(&cli_wrapper, "console.log('wrapper');\n").expect("cli wrapper");
        fs::write(
            package_root.join("package.json"),
            r#"{"bin":{"claude":"bin/claude.exe"}}"#,
        )
        .expect("package.json");

        let nodejs_dir = temp.path().join("nodejs");
        fs::create_dir_all(&nodejs_dir).expect("nodejs dir");
        let node_exe = nodejs_dir.join("node.exe");
        fs::write(&node_exe, b"MZ\x00").expect("node.exe");

        let mut config = sample_versioned_launch_config();
        config.command = "claude".to_string();
        config.args = vec!["--print".to_string()];
        config.windows_shell = Some(gwt_agent::WindowsShellKind::CommandPrompt);
        config.env_vars.insert(
            "PATH".to_string(),
            std::env::join_paths([bun_bin_dir.as_path(), nodejs_dir.as_path()])
                .expect("join PATH")
                .to_string_lossy()
                .into_owned(),
        );
        config
            .env_vars
            .insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        config.env_vars.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        apply_windows_host_shell_wrapper(&mut config).expect("wrap command prompt");

        assert_eq!(config.command, "cmd.exe");
        let expression = config
            .env_vars
            .get(WINDOWS_HOST_SHELL_EXPRESSION_ENV)
            .expect("cmd wrapper expression");
        assert!(
            expression.contains(&node_exe.display().to_string()),
            "expected resolved node.exe in wrapper expression, got: {expression}"
        );
        assert!(
            expression.contains(&cli_wrapper.display().to_string()),
            "expected cli-wrapper.cjs in wrapper expression, got: {expression}"
        );
        assert!(
            !expression.contains("call claude --print"),
            "wrapper must not direct-launch claude from PATH: {expression}"
        );
        assert!(
            !expression.contains(&placeholder_stub.display().to_string()),
            "wrapper must not direct-launch the placeholder stub: {expression}"
        );
    }

    #[test]
    fn package_runner_probe_forwards_failed_stderr_to_agent_console() {
        let hub = gwt_core::process_console::ProcessConsoleHub::new();
        let (command, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec![
                    "/C".to_string(),
                    "echo probe boom 1>&2 & exit /b 1".to_string(),
                ],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "echo probe boom >&2; exit 1".to_string()],
            )
        };

        let outcome = probe_host_package_runner_with_timeout_and_hub(
            PackageRunnerProbeRequest {
                command: &command,
                args,
                env_vars: &HashMap::new(),
                remove_env: &[],
                cwd: None,
                timeout: Duration::from_secs(2),
                poll_interval: Duration::from_millis(10),
            },
            &hub,
        );

        assert!(!outcome.success);
        let lines = hub.snapshot_kind(gwt_core::process_console::ProcessKind::AgentBootstrap);
        assert!(
            lines.iter().any(|line| {
                line.stream == gwt_core::process_console::ProcessStream::Stderr
                    && line.message.contains("probe boom")
            }),
            "expected failed probe stderr in agent console lines: {lines:?}",
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_npx_cache_corruption_detection_requires_verified_old_binary_signature() {
        let temp = tempdir().expect("tempdir");
        let npx_base = temp
            .path()
            .join("Local Cache With Spaces")
            .join("npm-cache")
            .join("_npx");
        let npx_root = npx_base.join("97540b0888a2deac");
        let bin_dir = npx_root
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        fs::write(bin_dir.join("claude.exe.old.1779939935247"), "binary")
            .expect("write old binary marker");
        let missing_binary = bin_dir.join("claude.exe");
        let stderr = format!(
            "'\"{}\"' is not recognized as an internal or external command",
            missing_binary.display()
        );

        let candidate = detect_windows_npx_cache_corruption(&stderr, &npx_base)
            .expect("corrupt npx cache should be detected");

        assert_eq!(candidate.npx_root, npx_root);
        assert_eq!(candidate.missing_binary, missing_binary);

        fs::write(&candidate.missing_binary, "restored binary").expect("write expected binary");
        assert!(
            detect_windows_npx_cache_corruption(&stderr, &npx_base).is_none(),
            "existing expected binary must not be treated as repairable",
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_npx_cache_corruption_detection_rejects_paths_outside_local_npx_root() {
        let temp = tempdir().expect("tempdir");
        let npx_base = temp.path().join("npm-cache").join("_npx");
        let outside_root = temp.path().join("other-cache").join("_npx").join("abc");
        let bin_dir = outside_root
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        fs::write(bin_dir.join("claude.exe.old.1779939935247"), "binary")
            .expect("write old binary marker");
        let stderr = format!(
            "'\"{}\"' is not recognized as an internal or external command",
            bin_dir.join("claude.exe").display()
        );

        assert!(
            detect_windows_npx_cache_corruption(&stderr, &npx_base).is_none(),
            "paths outside the verified npm _npx root must never be repaired",
        );
    }

    #[cfg(windows)]
    #[test]
    fn checked_host_package_runner_fallback_repairs_corrupt_npx_cache_once_before_switching() {
        let temp = tempdir().expect("tempdir");
        let npx_base = temp.path().join("npm-cache").join("_npx");
        let npx_root = npx_base.join("97540b0888a2deac");
        let bin_dir = npx_root
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        fs::write(bin_dir.join("claude.exe.old.1779939935247"), "binary")
            .expect("write old binary marker");
        let stderr = format!(
            "'\"{}\"' is not recognized as an internal or external command",
            bin_dir.join("claude.exe").display()
        );
        let mut config = sample_versioned_launch_config();
        let mut probe_calls = Vec::new();
        let mut repair_calls = Vec::new();

        let report = apply_host_package_runner_fallback_checked_with_probe_and_repair(
            &mut config,
            "npx".to_string(),
            Some(npx_base.clone()),
            |command, args, _env, _remove_env, _cwd| {
                probe_calls.push((command.to_string(), args.clone()));
                match probe_calls.len() {
                    1 => PackageRunnerProbeOutcome::failure_with_stderr("bunx unavailable"),
                    2 => PackageRunnerProbeOutcome::failure_with_stderr(&stderr),
                    3 => PackageRunnerProbeOutcome::success(),
                    _ => panic!("unexpected extra probe call: {probe_calls:?}"),
                }
            },
            |candidate| {
                repair_calls.push(candidate.npx_root.clone());
                fs::remove_dir_all(&candidate.npx_root).expect("remove corrupt npx root");
                Ok(())
            },
        )
        .expect("corrupt npx cache should be repaired");

        assert!(report.switched_to_fallback);
        assert!(report.repaired_npx_cache);
        assert_eq!(repair_calls, vec![npx_root]);
        assert_eq!(probe_calls.len(), 3);
        assert_eq!(probe_calls[1].0, "npx");
        assert_eq!(
            probe_calls[1].1,
            vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
                "--version".to_string(),
            ],
        );
        assert_eq!(config.command, "npx");
        assert_eq!(
            config.args,
            vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
                "--print".to_string(),
            ],
        );
    }

    #[cfg(windows)]
    #[test]
    fn checked_host_package_runner_fallback_fails_before_spawn_when_npx_repair_fails() {
        let temp = tempdir().expect("tempdir");
        let npx_base = temp.path().join("npm-cache").join("_npx");
        let npx_root = npx_base.join("97540b0888a2deac");
        let bin_dir = npx_root
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code")
            .join("bin");
        fs::create_dir_all(&bin_dir).expect("create bin dir");
        fs::write(bin_dir.join("claude.exe.old.1779939935247"), "binary")
            .expect("write old binary marker");
        let stderr = format!(
            "'\"{}\"' is not recognized as an internal or external command",
            bin_dir.join("claude.exe").display()
        );
        let mut config = sample_versioned_launch_config();
        let original_command = config.command.clone();
        let mut repair_calls = 0;

        let error = apply_host_package_runner_fallback_checked_with_probe_and_repair(
            &mut config,
            "npx".to_string(),
            Some(npx_base),
            |command, _args, _env, _remove_env, _cwd| {
                if command.eq_ignore_ascii_case("bunx") {
                    PackageRunnerProbeOutcome::failure_with_stderr("bunx unavailable")
                } else {
                    PackageRunnerProbeOutcome::failure_with_stderr(&stderr)
                }
            },
            |_candidate| {
                repair_calls += 1;
                Err("access denied".to_string())
            },
        )
        .expect_err("repair failure should stop before agent spawn");

        assert_eq!(repair_calls, 1);
        assert_eq!(config.command, original_command);
        assert!(error.contains("Failed to repair npm npx cache"));
        assert!(error.contains("access denied"));
        assert!(error.contains(&npx_root.display().to_string()));
    }

    #[cfg(windows)]
    #[test]
    fn checked_host_package_runner_fallback_does_not_repair_unrelated_npx_failure() {
        let temp = tempdir().expect("tempdir");
        let npx_base = temp.path().join("npm-cache").join("_npx");
        let mut config = sample_versioned_launch_config();
        let mut repair_calls = 0;

        let error = apply_host_package_runner_fallback_checked_with_probe_and_repair(
            &mut config,
            "npx".to_string(),
            Some(npx_base),
            |command, _args, _env, _remove_env, _cwd| {
                if command.eq_ignore_ascii_case("bunx") {
                    PackageRunnerProbeOutcome::failure_with_stderr("bunx unavailable")
                } else {
                    PackageRunnerProbeOutcome::failure_with_stderr("registry timeout")
                }
            },
            |_candidate| {
                repair_calls += 1;
                Ok(())
            },
        )
        .expect_err("unrelated npx failure should fail before agent spawn");

        assert_eq!(repair_calls, 0);
        assert!(error.contains("npx package-runner probe failed"));
        assert!(error.contains("registry timeout"));
    }

    // Issue #2948 — non-Windows host launches must decide the package runner by
    // *binary availability* only, never by executing `<runner> <pkg> --version`
    // (a cold first-run download exceeds the probe budget, times out, and aborts
    // the launch with an error card instead of showing the raw TTY download).

    #[cfg(not(windows))]
    fn write_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(path, "#!/bin/sh\nexit 1\n").expect("write executable");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod +x");
    }

    #[cfg(not(windows))]
    #[test]
    fn runner_binary_available_trusts_existing_absolute_path() {
        let temp = tempdir().expect("tempdir");
        let bin = temp.path().join("bunx");
        write_executable(&bin);
        let env = HashMap::new();
        assert!(runner_binary_available(bin.to_str().unwrap(), &env));
        let missing = temp.path().join("does-not-exist");
        assert!(!runner_binary_available(missing.to_str().unwrap(), &env));
    }

    #[cfg(not(windows))]
    #[test]
    fn runner_binary_available_resolves_bare_name_via_env_path() {
        let temp = tempdir().expect("tempdir");
        write_executable(&temp.path().join("bunx"));
        let env = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);
        assert!(runner_binary_available("bunx", &env));
        assert!(!runner_binary_available("npx", &env));
    }

    #[cfg(not(windows))]
    #[test]
    fn host_package_runner_binary_outcome_succeeds_from_existence_without_executing() {
        // /bin/sh exists but is not a package runner; success comes purely from
        // binary existence, proving no `<runner> <pkg> --version` is executed.
        let env = HashMap::new();
        let outcome = host_package_runner_binary_outcome("/bin/sh", &env);
        assert!(outcome.success);
        assert!(!outcome.timed_out);

        let missing = host_package_runner_binary_outcome("/no/such/runner-xyz", &env);
        assert!(!missing.success);
        assert!(!missing.timed_out);
    }

    #[cfg(not(windows))]
    #[test]
    fn host_launch_keeps_bunx_when_binary_resolves_and_never_aborts() {
        let temp = tempdir().expect("tempdir");
        // A "bunx" that fails (`exit 1`) if executed — proving the launch path
        // only checks existence and keeps bunx, where the old execution probe
        // would reject it and fall through to an abort.
        let bunx = temp.path().join("bunx");
        write_executable(&bunx);
        let mut config = sample_versioned_launch_config();
        config.command = bunx.display().to_string();
        config.env_vars = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);

        let report = apply_host_package_runner_fallback_checked(&mut config)
            .expect("binary-availability resolution must never abort the launch");

        assert!(!report.switched_to_fallback);
        assert_eq!(config.command, bunx.display().to_string());
    }

    #[cfg(not(windows))]
    #[test]
    fn host_launch_switches_to_npx_when_bunx_absent_but_npx_present() {
        let temp = tempdir().expect("tempdir");
        write_executable(&temp.path().join("npx"));
        let mut config = sample_versioned_launch_config();
        config.command = "bunx".to_string(); // bunx is NOT in the temp PATH
        config.env_vars = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);

        let report = apply_host_package_runner_fallback_checked(&mut config)
            .expect("resolution must never abort the launch");

        assert!(report.switched_to_fallback);
        // Issue #2981: the fallback now resolves the npx executable on PATH
        // (mirroring the primary runner) instead of emitting a bare `"npx"`.
        assert_eq!(
            config.command,
            temp.path().join("npx").display().to_string()
        );
        assert_eq!(config.args.first().map(String::as_str), Some("--yes"));
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
    fn build_shell_process_launch_normalizes_windows_host_cwd() {
        let mut config = ShellLaunchConfig {
            working_dir: Some(PathBuf::from(
                r"Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\20260525-0919",
            )),
            branch: None,
            base_branch: None,
            display_name: "Shell".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            windows_shell: Some(gwt_agent::WindowsShellKind::CommandPrompt),
            env_vars: HashMap::new(),
            remove_env: Vec::new(),
        };

        let launch = build_shell_process_launch(Path::new("/tmp/fallback"), &mut config)
            .expect("shell launch");

        let expected = PathBuf::from(r"E:\gwt\work\20260525-0919");
        assert_eq!(config.working_dir, Some(expected.clone()));
        assert_eq!(launch.cwd, Some(expected));
        assert_eq!(
            launch.env.get("GWT_PROJECT_ROOT").map(String::as_str),
            Some(r"E:\gwt\work\20260525-0919")
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
