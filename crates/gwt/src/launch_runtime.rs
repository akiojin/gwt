use super::*;

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

/// Resolve a working directory for an ephemeral intake launch (SPEC-3214
/// T-004): materialize a detached `.intake-*` worktree at `base_ref` and set
/// `working_dir`. Unlike [`resolve_launch_worktree_request`] this never creates
/// a branch — the intake worktree hosts a short-lived session and is removed
/// when the session ends. `working_dir` already set is a no-op (idempotent /
/// reuse). Collisions with existing worktrees are avoided by suffixing.
pub fn resolve_ephemeral_launch_worktree(
    repo_path: &Path,
    base_ref: Option<&str>,
    working_dir: &mut Option<PathBuf>,
    env_vars: &mut HashMap<String, String>,
) -> Result<(), String> {
    if working_dir.is_some() {
        return Ok(());
    }

    let main_repo_path =
        gwt_git::worktree::main_worktree_root(repo_path).map_err(|err| err.to_string())?;
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    let worktrees = manager.list().map_err(|err| err.to_string())?;

    let layout_root = main_repo_path.parent().unwrap_or(main_repo_path.as_path());
    let preferred_path = layout_root.join(INTAKE_WORKTREE_PREFIX);
    let worktree_path = first_available_worktree_path(&preferred_path, &worktrees)
        .ok_or_else(|| "failed to resolve available intake worktree path".to_string())?;

    // Default to HEAD: `git worktree add --detach <path> HEAD` always resolves
    // in a repo with commits. Callers (Phase 3 intake launch) pass an explicit
    // base ref such as `origin/develop` when they need a specific base.
    let base_ref = base_ref.unwrap_or("HEAD");
    manager
        .create_detached(base_ref, &worktree_path)
        .map_err(|err| err.to_string())?;

    set_worktree_launch_path(working_dir, env_vars, &worktree_path);
    Ok(())
}

fn is_start_work_branch_name(branch_name: &str) -> bool {
    branch_name
        .strip_prefix("work/")
        .is_some_and(|name| !name.is_empty())
}

/// Reap orphaned ephemeral intake worktrees at startup (SPEC-3214 T-006).
///
/// A crash between an intake launch and its session-end cleanup leaves a
/// detached `.intake-*` worktree behind. On startup no intake session is live,
/// so every `.intake-*` worktree is an orphan: remove the clean ones and keep
/// the dirty ones (uncommitted work is never destroyed). Bounded by
/// `max_removals` so a pathological pile-up cannot stall startup. Returns the
/// number removed. Never errors — best-effort recovery.
pub fn prune_orphan_intake_worktrees(repo_path: &Path, max_removals: usize) -> usize {
    let Ok(main_repo_path) = gwt_git::worktree::main_worktree_root(repo_path) else {
        return 0;
    };
    let manager = gwt_git::WorktreeManager::new(&main_repo_path);
    let Ok(worktrees) = manager.list() else {
        return 0;
    };

    let mut removed = 0;
    for worktree in worktrees {
        if removed >= max_removals {
            break;
        }
        if !is_ephemeral_intake_worktree(&worktree.path) {
            continue;
        }
        // codex #3236 P2: only reap the branchless intake worktrees this feature
        // creates — a real branch worktree a user happens to name `.intake-*`
        // has a branch and must be left alone (mirrors is_ephemeral_intake_session).
        if worktree.branch.is_some() {
            continue;
        }
        let worktree_path = worktree.path.clone();
        match manager.ephemeral_worktree_has_local_work_with(&worktree.path, |entry| {
            intake_hook_config_is_disposable(&worktree_path, entry)
        }) {
            Ok(false) => {
                if manager.remove_force(&worktree.path).is_ok() {
                    removed += 1;
                }
            }
            // Has local work or unknown → keep it (fail closed).
            _ => {
                tracing::warn!(
                    worktree_path = %worktree.path.display(),
                    "keeping orphaned intake worktree with local work (changes, ignored files, or commits)"
                );
            }
        }
    }
    if removed > 0 {
        let _ = manager.prune();
    }
    removed
}

pub fn resolve_launch_worktree(
    repo_path: &Path,
    config: &mut gwt_agent::LaunchConfig,
) -> Result<(), String> {
    // SPEC-3214: an ephemeral intake launch resolves a detached throwaway
    // worktree instead of creating/reusing a branch worktree.
    if config.is_ephemeral {
        resolve_ephemeral_launch_worktree(
            repo_path,
            config.ephemeral_base_ref.as_deref(),
            &mut config.working_dir,
            &mut config.env_vars,
        )?;
        normalize_launch_config_working_dir(config);
        return Ok(());
    }
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
        // SPEC-3151 FR-010: an explicit command override (e.g. the OpenCode
        // setup launcher `bunx opencode-ai@latest auth login`) replaces the
        // detected interactive shell. `detect_shell_program()` is only called
        // when there is no override.
        let shell = if let Some(command) = config.command_override.clone() {
            gwt::ShellProgram {
                command,
                args: config.command_args_override.clone().unwrap_or_default(),
            }
        } else {
            let windows_shell = if cfg!(windows) {
                config.windows_shell
            } else {
                None
            };
            match windows_shell {
                Some(windows_shell) => gwt::ShellProgram {
                    command: windows_shell_process_command(windows_shell).to_string(),
                    args: interactive_windows_shell_args(windows_shell),
                },
                None => detect_shell_program().map_err(|error| error.to_string())?,
            }
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
    let runtime =
        gwt_docker::detect::ResolvedContainerRuntime::resolve(&docker_binary_for_launch())?;
    ensure_docker_launch_runtime_ready_for_runtime(&runtime)?;
    crate::docker_launch::ensure_docker_gwt_binary_setup_for_runtime(&launch, runtime.kind())?;
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
        command: runtime.binary().to_string(),
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

    let normalized = gwt_terminal::pty::normalize_command_for_windows_host_shell(
        &config.command,
        &config.args,
        &config.env_vars,
        &config.remove_env,
    )?;
    config.env_vars = normalized.env;
    // Share the PTY path's pre-spawn backstop: if resolution still landed on a
    // non-PE placeholder stub (no cli-wrapper/native to redirect to), refuse here
    // rather than embed it into the shell expression and surface the Windows
    // 16-bit dialog from inside cmd/PowerShell.
    if let Some(reason) = gwt_terminal::pty::reject_non_pe_executable(&normalized.command) {
        return Err(reason);
    }
    let (command, args) = wrap_windows_host_shell_command(
        shell,
        &normalized.command,
        &normalized.args,
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
    let requires_call = Path::new(command)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        });
    let mut parts = Vec::with_capacity(args.len() + 1 + usize::from(requires_call));
    if requires_call {
        parts.push("call".to_string());
    }
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
    probe: F,
) -> bool
where
    F: FnMut(&str, Vec<String>, &HashMap<String, String>, &[String], Option<PathBuf>) -> bool,
{
    gwt_agent::apply_host_package_runner_fallback_with_probe(config, fallback_executable, probe)
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
    gwt_agent::prepare::probe_host_runner_with_timeout(
        gwt_agent::HostRunnerProbeKind::Package,
        command,
        args,
        env_vars,
        remove_env,
        cwd,
        timeout,
        poll_interval,
    )
    .success
}

#[cfg(test)]
pub fn command_matches_runner(command: &str, runner: &str) -> bool {
    let path = Path::new(command);
    path.file_stem()
        .and_then(|stem| stem.to_str())
        .or_else(|| path.file_name().and_then(|name| name.to_str()))
        .is_some_and(|name| name.eq_ignore_ascii_case(runner))
}

pub fn ensure_docker_launch_runtime_ready_for_runtime(
    runtime: &gwt_docker::detect::ResolvedContainerRuntime,
) -> Result<(), String> {
    ensure_docker_launch_runtime_ready_with(runtime.binary(), || {
        gwt_docker::launch_preflight_for_resolved_runtime(runtime)
    })
}

fn ensure_docker_launch_runtime_ready_with(
    attempted_binary: &str,
    preflight: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    let path = std::env::var("PATH").unwrap_or_default();
    tracing::info!(
        target: "gwt::launch::preflight",
        runtime_target = "docker",
        attempted_binary,
        path = %path,
        "docker preflight started"
    );
    let result = preflight();
    match &result {
        Ok(()) => {
            tracing::info!(
                target: "gwt::launch::preflight",
                runtime_target = "docker",
                outcome = "ready",
                attempted_binary,
                "docker preflight completed"
            );
        }
        Err(error) => {
            tracing::error!(
                target: "gwt::launch::preflight",
                runtime_target = "docker",
                outcome = "failed",
                attempted_binary,
                path = %path,
                error = %error,
                "docker preflight failed"
            );
        }
    }
    result
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
    use std::fs;
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

    #[cfg(not(windows))]
    fn sample_direct_codex_launch_config(bin_dir: &Path) -> gwt_agent::LaunchConfig {
        write_executable(&bin_dir.join("bunx"));
        write_executable(&bin_dir.join("npx"));
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Codex)
            .working_dir(bin_dir)
            .model("gpt-5.6-codex")
            .session_mode(gwt_agent::SessionMode::Continue)
            .skip_permissions(true)
            .extra_arg("--search")
            .build();
        config.command = "/opt/homebrew/bin/codex".to_string();
        config.env_vars = HashMap::from([
            ("PATH".to_string(), bin_dir.display().to_string()),
            ("HOME".to_string(), bin_dir.display().to_string()),
        ]);
        config
    }

    fn probe_success() -> gwt_agent::HostRunnerProbeOutcome {
        gwt_agent::HostRunnerProbeOutcome {
            success: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            timed_out: false,
            error: None,
        }
    }

    fn probe_failure(detail: &str) -> gwt_agent::HostRunnerProbeOutcome {
        gwt_agent::HostRunnerProbeOutcome {
            success: false,
            exit_code: Some(1),
            stdout: String::new(),
            stderr: detail.to_string(),
            timed_out: false,
            error: None,
        }
    }

    #[cfg(not(windows))]
    #[test]
    fn checked_host_runner_falls_back_from_broken_direct_to_healthy_bunx() {
        let temp = tempdir().expect("tempdir");
        let mut config = sample_direct_codex_launch_config(temp.path());
        let original_args = config.args.clone();
        let mut probes = Vec::new();

        let report = gwt_agent::resolve_host_runner_health_checked_with_probe_and_repair(
            &mut config,
            temp.path().join("npx").display().to_string(),
            None,
            |_kind, command, args, _env, _remove_env, _cwd| {
                probes.push((command.to_string(), args));
                match probes.len() {
                    1 => probe_failure("direct wrapper vendor binary missing"),
                    2 => probe_success(),
                    _ => panic!("unexpected probe sequence: {probes:?}"),
                }
            },
            |_candidate| panic!("cache repair must not run"),
        )
        .expect("healthy bunx fallback");

        assert!(report.switched_to_fallback);
        assert_eq!(probes[0].0, "/opt/homebrew/bin/codex");
        assert_eq!(probes[0].1, vec!["--version".to_string()]);
        assert_eq!(probes[1].0, temp.path().join("bunx").display().to_string());
        assert_eq!(probes[1].1, vec!["--version".to_string()]);
        assert_eq!(config.command, probes[1].0);
        let package_index = config
            .args
            .iter()
            .position(|arg| arg == "@openai/codex@latest")
            .expect("latest package prefix");
        assert_eq!(&config.args[package_index + 1..], original_args.as_slice());
    }

    #[cfg(not(windows))]
    #[test]
    fn checked_host_runner_falls_back_from_broken_bunx_to_healthy_npx() {
        let temp = tempdir().expect("tempdir");
        let mut config = sample_direct_codex_launch_config(temp.path());
        let original_args = config.args.clone();
        let mut probes = Vec::new();

        let report = gwt_agent::resolve_host_runner_health_checked_with_probe_and_repair(
            &mut config,
            temp.path().join("npx").display().to_string(),
            None,
            |_kind, command, args, _env, _remove_env, _cwd| {
                probes.push((command.to_string(), args));
                match probes.len() {
                    1 => probe_failure("direct wrapper vendor binary missing"),
                    2 => probe_failure("bunx unavailable"),
                    3 => probe_success(),
                    _ => panic!("unexpected probe sequence: {probes:?}"),
                }
            },
            |_candidate| panic!("cache repair must not run"),
        )
        .expect("healthy npx fallback");

        assert!(report.switched_to_fallback);
        assert_eq!(probes.len(), 3);
        assert_eq!(probes[0].1, vec!["--version".to_string()]);
        assert_eq!(probes[1].1, vec!["--version".to_string()]);
        assert_eq!(probes[2].0, temp.path().join("npx").display().to_string());
        assert_eq!(probes[2].1, vec!["--version".to_string()]);
        assert_eq!(config.command, probes[2].0);
        assert_eq!(config.args[0], "--yes");
        let package_index = config
            .args
            .iter()
            .position(|arg| arg == "@openai/codex@latest")
            .expect("latest package prefix");
        assert_eq!(&config.args[package_index + 1..], original_args.as_slice());
    }

    #[cfg(not(windows))]
    #[test]
    fn checked_host_runner_rejects_broken_direct_bunx_and_npx_without_mutating_launch() {
        let temp = tempdir().expect("tempdir");
        let mut config = sample_direct_codex_launch_config(temp.path());
        config
            .env_vars
            .insert("RUNNER_SENTINEL".into(), "keep".into());
        config.remove_env.push("REMOVE_SENTINEL".into());
        let original_command = config.command.clone();
        let original_args = config.args.clone();
        let original_config = format!("{config:?}");
        let mut probes = Vec::new();

        let error = gwt_agent::resolve_host_runner_health_checked_with_probe_and_repair(
            &mut config,
            temp.path().join("npx").display().to_string(),
            None,
            |_kind, command, args, _env, _remove_env, _cwd| {
                probes.push((command.to_string(), args));
                probe_failure(match probes.len() {
                    1 => "direct wrapper vendor binary missing",
                    2 => "bunx unavailable",
                    3 => "npx unavailable",
                    _ => panic!("unexpected probe sequence: {probes:?}"),
                })
            },
            |_candidate| panic!("cache repair must not run"),
        )
        .expect_err("all broken runners must stop before dispatch");

        assert_eq!(probes.len(), 3);
        assert_eq!(config.command, original_command);
        assert_eq!(config.args, original_args);
        assert_eq!(format!("{config:?}"), original_config);
        assert!(error.contains("direct wrapper vendor binary missing"));
        assert!(error.contains("npx unavailable"));
    }

    #[test]
    fn checked_host_runner_uses_descriptor_version_argv_for_copilot() {
        let mut config = gwt_agent::AgentLaunchBuilder::new(gwt_agent::AgentId::Copilot).build();
        config.command = "/usr/local/bin/gh".to_string();
        let original_args = config.args.clone();
        let mut probes = Vec::new();

        let report = gwt_agent::resolve_host_runner_health_checked_with_probe_and_repair(
            &mut config,
            "npx".to_string(),
            None,
            |_kind, command, args, _env, _remove_env, _cwd| {
                probes.push((command.to_string(), args));
                probe_success()
            },
            |_candidate| panic!("cache repair must not run"),
        )
        .expect("healthy Copilot direct runner");

        assert!(!report.switched_to_fallback);
        assert_eq!(probes.len(), 1);
        assert_eq!(probes[0].0, "/usr/local/bin/gh");
        assert_eq!(
            probes[0].1,
            vec!["copilot".to_string(), "--version".to_string()]
        );
        assert_eq!(config.args, original_args);
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
        fs::copy(
            std::env::current_exe().expect("current test executable"),
            &node_exe,
        )
        .expect("copy real node PE fixture");

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

    #[cfg(windows)]
    #[test]
    fn command_prompt_agent_wrapper_preserves_inner_cmd_expression_env() {
        let temp = tempdir().expect("tempdir");
        let bin = temp.path().join("Program Files").join("npm bin");
        fs::create_dir_all(&bin).expect("cmd shim directory");
        let shim = bin.join("npx.cmd");
        fs::write(&shim, "@echo off\r\n").expect("cmd shim");

        let mut config = sample_versioned_launch_config();
        config.command = "npx".to_string();
        config.args = vec!["a&b".to_string()];
        config.windows_shell = Some(gwt_agent::WindowsShellKind::CommandPrompt);
        config
            .env_vars
            .insert("PATH".to_string(), bin.display().to_string());
        config
            .env_vars
            .insert("PATHEXT".to_string(), ".CMD".to_string());

        apply_windows_host_shell_wrapper(&mut config).expect("wrap command prompt");

        let inner = config
            .env_vars
            .get(gwt_core::process::WINDOWS_CMD_WRAPPER_EXPRESSION_ENV)
            .expect("resolver-owned inner cmd expression");
        assert_eq!(inner, &format!("\"{}\" \"a&b\"", shim.display()));
        let outer = config
            .env_vars
            .get(WINDOWS_HOST_SHELL_EXPRESSION_ENV)
            .expect("outer host-shell expression");
        assert!(
            outer.contains(gwt_core::process::WINDOWS_CMD_WRAPPER_EXPRESSION_ENV),
            "{outer}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn command_prompt_agent_wrapper_rejects_unredirectable_placeholder_stub() {
        // A placeholder bin with NO cli-wrapper.cjs and NO *-win32-x64 native:
        // resolution cannot redirect, so the host-shell wrapper must refuse with
        // an actionable error rather than embed the non-PE stub into the shell
        // expression (which would raise the Windows 16-bit dialog from cmd).
        let temp = tempdir().expect("tempdir");
        let package_root = temp
            .path()
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        let bin_dir = package_root.join("bin");
        fs::create_dir_all(&bin_dir).expect("bin dir");
        let placeholder_stub = bin_dir.join("claude.exe");
        fs::write(&placeholder_stub, "Error: native binary not installed\n").expect("stub");
        fs::write(
            package_root.join("package.json"),
            r#"{"bin":{"claude":"bin/claude.exe"}}"#,
        )
        .expect("package.json");

        let mut config = sample_versioned_launch_config();
        config.command = placeholder_stub.display().to_string();
        config.windows_shell = Some(gwt_agent::WindowsShellKind::CommandPrompt);
        config
            .env_vars
            .insert("PATHEXT".to_string(), ".COM;.EXE;.BAT;.CMD".to_string());
        config.env_vars.insert(
            "USERPROFILE".to_string(),
            temp.path().join("no_bun").display().to_string(),
        );

        let err = match apply_windows_host_shell_wrapper(&mut config) {
            Ok(()) => panic!("host-shell wrapper must reject a non-PE placeholder stub"),
            Err(e) => e,
        };
        assert!(
            err.contains("native-binary placeholder without a safe wrapper"),
            "expected actionable non-PE error, got: {err}"
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

        let candidate = gwt_agent::prepare::detect_windows_npx_cache_corruption(&stderr, &npx_base)
            .expect("corrupt npx cache should be detected");

        assert_eq!(candidate.npx_root, npx_root);
        assert_eq!(candidate.missing_binary, missing_binary);

        fs::write(&candidate.missing_binary, "restored binary").expect("write expected binary");
        assert!(
            gwt_agent::prepare::detect_windows_npx_cache_corruption(&stderr, &npx_base).is_none(),
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
            gwt_agent::prepare::detect_windows_npx_cache_corruption(&stderr, &npx_base).is_none(),
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

        let report = gwt_agent::resolve_host_runner_health_checked_with_probe_and_repair(
            &mut config,
            "npx".to_string(),
            Some(npx_base.clone()),
            |_kind, command, args, _env, _remove_env, _cwd| {
                probe_calls.push((command.to_string(), args.clone()));
                match probe_calls.len() {
                    1 => gwt_agent::HostRunnerProbeOutcome::failure_with_stderr("bunx unavailable"),
                    2 => gwt_agent::HostRunnerProbeOutcome::failure_with_stderr(&stderr),
                    3 => gwt_agent::HostRunnerProbeOutcome::success(),
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
        assert_eq!(probe_calls[1].1, vec!["--version".to_string()]);
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

        let error = gwt_agent::resolve_host_runner_health_checked_with_probe_and_repair(
            &mut config,
            "npx".to_string(),
            Some(npx_base),
            |_kind, command, _args, _env, _remove_env, _cwd| {
                if command.eq_ignore_ascii_case("bunx") {
                    gwt_agent::HostRunnerProbeOutcome::failure_with_stderr("bunx unavailable")
                } else {
                    gwt_agent::HostRunnerProbeOutcome::failure_with_stderr(&stderr)
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

        let error = gwt_agent::resolve_host_runner_health_checked_with_probe_and_repair(
            &mut config,
            "npx".to_string(),
            Some(npx_base),
            |_kind, command, _args, _env, _remove_env, _cwd| {
                if command.eq_ignore_ascii_case("bunx") {
                    gwt_agent::HostRunnerProbeOutcome::failure_with_stderr("bunx unavailable")
                } else {
                    gwt_agent::HostRunnerProbeOutcome::failure_with_stderr("registry timeout")
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

    #[cfg(windows)]
    #[test]
    fn checked_host_package_runner_fallback_rejects_npx_timeout_without_mutating_launch() {
        let temp = tempdir().expect("tempdir");
        let npx_base = temp.path().join("npm-cache").join("_npx");
        let mut config = sample_versioned_launch_config();
        config
            .env_vars
            .insert("RUNNER_API_TOKEN".to_string(), "must-not-leak".to_string());
        config.remove_env.push("REMOVE_SENTINEL".to_string());
        let original = format!("{config:?}");
        let mut probe_calls = Vec::new();
        let mut repair_calls = 0;

        let error = gwt_agent::resolve_host_runner_health_checked_with_probe_and_repair(
            &mut config,
            "npx".to_string(),
            Some(npx_base),
            |_kind, command, args, _env, _remove_env, _cwd| {
                probe_calls.push((command.to_string(), args.clone()));
                match probe_calls.len() {
                    1 => gwt_agent::HostRunnerProbeOutcome::failure_with_stderr("bunx unavailable"),
                    2 => gwt_agent::HostRunnerProbeOutcome::timeout(),
                    _ => panic!("unexpected extra probe call: {probe_calls:?}"),
                }
            },
            |_candidate| {
                repair_calls += 1;
                Ok(())
            },
        )
        .expect_err("npx probe timeout must stop before PTY spawn");

        assert_eq!(repair_calls, 0);
        assert_eq!(probe_calls.len(), 2);
        assert_eq!(format!("{config:?}"), original);
        assert!(error.contains("npx"));
        assert!(error.contains("@anthropic-ai/claude-code@latest"));
        assert!(error.contains("probe timed out"));
        assert!(!error.contains("must-not-leak"));
    }

    // Issue #2948 reconciliation — non-Windows host launches execute only the
    // package runner's own bounded `--version` probe. They must never execute
    // `<runner> <pkg> --version`, which can trigger a cold package download.

    #[cfg(not(windows))]
    fn write_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::write(
            path,
            "#!/bin/sh\n[ \"$1\" = \"--version\" ] || exit 1\nprintf '1.2.3\\n'\n",
        )
        .expect("write executable");
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod +x");
    }

    #[cfg(not(windows))]
    #[test]
    fn host_launch_keeps_bunx_when_runner_version_probe_succeeds() {
        let temp = tempdir().expect("tempdir");
        let bunx = temp.path().join("bunx");
        write_executable(&bunx);
        let mut config = sample_versioned_launch_config();
        config.command = bunx.display().to_string();
        config.env_vars = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);
        config.working_dir = Some(temp.path().to_path_buf());

        let report = gwt_agent::resolve_host_runner_health_checked(&mut config)
            .expect("runner version probe should keep bunx healthy");

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
        config.working_dir = Some(temp.path().to_path_buf());

        let report = gwt_agent::resolve_host_runner_health_checked(&mut config)
            .expect("healthy npx version probe should select the fallback");

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
            command_override: None,
            command_args_override: None,
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

    #[cfg(unix)]
    #[test]
    fn docker_shell_launch_pins_one_stateful_runtime_resolution() {
        use std::os::unix::fs::PermissionsExt;

        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let project = temp.path().join("project");
        let home = temp.path().join("home");
        fs::create_dir_all(&project).expect("project dir");
        fs::create_dir_all(&home).expect("home dir");
        fs::write(
            project.join("docker-compose.yml"),
            "services:\n  app:\n    image: alpine:3.19\n    working_dir: /workspace/app\n",
        )
        .expect("write compose");

        let wrapper = temp.path().join("stateful-container-wrapper");
        fs::write(
            &wrapper,
            r#"#!/bin/sh
printf '%s\n' "$*" >> "${0}.calls"
if [ "$1" = "--version" ]; then
  count=0
  if [ -f "${0}.version-count" ]; then
    read count < "${0}.version-count"
  fi
  count=$((count + 1))
  printf '%s\n' "$count" > "${0}.version-count"
  if [ "$count" -eq 1 ]; then
    printf 'Docker version 28.3.0, build test\n'
  else
    printf 'podman version 5.4.2\n'
  fi
  exit 0
fi
for arg in "$@"; do
  if [ "$arg" = "ps" ]; then
    printf 'app\trunning\n'
    exit 0
  fi
done
exit 0
"#,
        )
        .expect("write stateful wrapper");
        let mut permissions = fs::metadata(&wrapper)
            .expect("wrapper metadata")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&wrapper, permissions).expect("chmod stateful wrapper");

        let _docker_bin =
            gwt_core::test_support::ScopedEnvVar::set("GWT_DOCKER_BIN", wrapper.as_os_str());
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", home.as_os_str());
        let mut config = ShellLaunchConfig {
            working_dir: Some(project.clone()),
            branch: None,
            base_branch: None,
            display_name: "Docker shell".to_string(),
            runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
            docker_service: Some("app".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            windows_shell: None,
            env_vars: HashMap::new(),
            remove_env: Vec::new(),
            command_override: None,
            command_args_override: None,
        };

        let launch =
            build_shell_process_launch(&project, &mut config).expect("Docker shell launch");

        assert_eq!(launch.command, wrapper.display().to_string());
        assert!(
            launch
                .args
                .ends_with(&["app".to_string(), "bash".to_string()]),
            "unexpected Docker shell argv: {:?}",
            launch.args
        );
        let calls = fs::read_to_string(wrapper.with_extension("calls"))
            .expect("read runtime wrapper calls");
        assert_eq!(
            calls.lines().filter(|call| *call == "--version").count(),
            1,
            "the shell launch must resolve its stateful runtime exactly once"
        );
        let managed_override = fs::read_to_string(project.join("docker-compose.gwt.override.yml"))
            .expect("read managed override");
        assert!(
            managed_override.contains(gwt_docker::DOCKER_HOST_GATEWAY_EXTRA_HOST),
            "the first resolved Docker kind must stay pinned through setup"
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
