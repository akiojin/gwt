//! gwt - Git Worktree Manager CLI

use chrono::Utc;
use clap::Parser;
use gwt_core::agent::codex::{codex_default_args, codex_skip_permissions_flag};
use gwt_core::agent::get_command_version;
use gwt_core::config::{save_session_entry, Settings, ToolSessionEntry};
use gwt_core::error::GwtError;
use gwt_core::worktree::WorktreeManager;
use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};
use tracing::{error, info, warn};

mod cli;
mod tui;

use cli::{Cli, Commands, OutputFormat};
use tui::{AgentLaunchConfig, CodingAgent, ExecutionMode, TuiEntryContext};

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), GwtError> {
    let cli = Cli::parse();

    // Check if git is available
    if !check_git_available() {
        return Err(GwtError::GitNotFound);
    }

    // Determine repo root
    let repo_root = cli
        .repo
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    // Load settings
    let settings = Settings::load(&repo_root).unwrap_or_default();

    // Initialize logging
    // Note: settings.log_dir() already includes workspace name, so we pass empty workspace
    let log_config = gwt_core::logging::LogConfig {
        debug: cli.debug || std::env::var("GWT_DEBUG").is_ok(),
        log_dir: settings.log_dir(&repo_root),
        workspace: String::new(),
        ..Default::default()
    };
    gwt_core::logging::init_logger(&log_config)?;

    info!(
        repo_root = %repo_root.display(),
        debug = log_config.debug,
        "gwt started"
    );

    match cli.command {
        Some(cmd) => handle_command(cmd, &repo_root, &settings),
        None => {
            // Interactive TUI mode
            if let Ok(manager) = WorktreeManager::new(&repo_root) {
                let _ = manager.auto_cleanup_orphans();
            }
            let mut entry: Option<TuiEntryContext> = None;
            loop {
                let selection = tui::run_with_context(entry.take())?;
                match selection {
                    Some(launch_config) => match launch_coding_agent(launch_config) {
                        Ok(AgentExitKind::Success) => {
                            entry = Some(TuiEntryContext::success(
                                "Session completed successfully.".to_string(),
                            ));
                        }
                        Ok(AgentExitKind::Interrupted) => {
                            entry =
                                Some(TuiEntryContext::warning("Session interrupted.".to_string()));
                        }
                        Err(err) => {
                            entry = Some(TuiEntryContext::error(err.to_string()));
                        }
                    },
                    None => break,
                }
            }
            Ok(())
        }
    }
}

fn handle_command(cmd: Commands, repo_root: &PathBuf, settings: &Settings) -> Result<(), GwtError> {
    match cmd {
        Commands::List { format } => cmd_list(repo_root, format),
        Commands::Add { branch, new, base } => cmd_add(repo_root, &branch, new, base.as_deref()),
        Commands::Remove {
            target,
            force,
            delete_branch,
        } => cmd_remove(repo_root, &target, force, delete_branch),
        Commands::Switch { branch, new_window } => cmd_switch(repo_root, &branch, new_window),
        Commands::Clean { dry_run, prune } => cmd_clean(repo_root, dry_run, prune),
        Commands::Logs { limit, follow: _ } => cmd_logs(repo_root, settings, limit),
        Commands::Serve { port, address } => cmd_serve(port, &address),
        Commands::Init { force } => cmd_init(repo_root, force),
        Commands::Lock { target, reason } => cmd_lock(repo_root, &target, reason.as_deref()),
        Commands::Unlock { target } => cmd_unlock(repo_root, &target),
        Commands::Repair { target } => cmd_repair(repo_root, target.as_deref()),
    }
}

fn cmd_list(repo_root: &PathBuf, format: OutputFormat) -> Result<(), GwtError> {
    let manager = WorktreeManager::new(repo_root)?;
    let worktrees = manager.list()?;

    match format {
        OutputFormat::Table => {
            println!(
                "{:<40} {:<30} {:<10} {:<8}",
                "PATH", "BRANCH", "STATUS", "CHANGES"
            );
            println!("{}", "-".repeat(88));
            for wt in &worktrees {
                let branch = wt
                    .branch
                    .clone()
                    .unwrap_or_else(|| "(detached)".to_string());
                let changes = if wt.has_changes { "dirty" } else { "clean" };
                println!(
                    "{:<40} {:<30} {:<10} {:<8}",
                    wt.path.display(),
                    branch,
                    wt.status,
                    changes
                );
            }
        }
        OutputFormat::Json => {
            let json = serde_json::json!(worktrees
                .iter()
                .map(|wt| {
                    serde_json::json!({
                        "path": wt.path.to_string_lossy(),
                        "branch": wt.branch,
                        "status": wt.status.to_string(),
                        "has_changes": wt.has_changes,
                        "has_unpushed": wt.has_unpushed,
                    })
                })
                .collect::<Vec<_>>());
            println!("{}", serde_json::to_string_pretty(&json).unwrap());
        }
        OutputFormat::Simple => {
            for wt in &worktrees {
                println!("{}", wt.path.display());
            }
        }
    }

    Ok(())
}

fn cmd_add(
    repo_root: &PathBuf,
    branch: &str,
    new_branch: bool,
    base: Option<&str>,
) -> Result<(), GwtError> {
    info!(
        category = "cli",
        command = "add",
        branch,
        new_branch,
        base = base.unwrap_or("HEAD"),
        "Executing add command"
    );

    let manager = WorktreeManager::new(repo_root)?;

    let wt = if new_branch {
        manager.create_new_branch(branch, base)?
    } else {
        manager.create_for_branch(branch)?
    };

    println!("Created worktree at: {}", wt.path.display());
    println!("Branch: {}", wt.display_name());

    Ok(())
}

fn cmd_remove(
    repo_root: &PathBuf,
    target: &str,
    force: bool,
    delete_branch: bool,
) -> Result<(), GwtError> {
    info!(
        category = "cli",
        command = "remove",
        target,
        force,
        delete_branch,
        "Executing remove command"
    );

    let manager = WorktreeManager::new(repo_root)?;

    // Find worktree by branch name or path
    let wt = manager
        .get_by_branch(target)?
        .or_else(|| {
            let path = PathBuf::from(target);
            manager.get_by_path(&path).ok().flatten()
        })
        .ok_or_else(|| GwtError::WorktreeNotFound {
            path: PathBuf::from(target),
        })?;

    let path = wt.path.clone();

    if delete_branch {
        manager.remove_with_branch(&path, force)?;
        println!("Removed worktree and branch: {}", target);
    } else {
        manager.remove(&path, force)?;
        println!("Removed worktree: {}", path.display());
    }

    Ok(())
}

fn cmd_switch(repo_root: &PathBuf, branch: &str, new_window: bool) -> Result<(), GwtError> {
    info!(
        category = "cli",
        command = "switch",
        branch,
        new_window,
        "Executing switch command"
    );

    let manager = WorktreeManager::new(repo_root)?;

    let wt = manager
        .get_by_branch(branch)?
        .ok_or_else(|| GwtError::WorktreeNotFound {
            path: PathBuf::from(branch),
        })?;

    if new_window {
        // Open in new terminal window (platform specific)
        #[cfg(target_os = "macos")]
        {
            std::process::Command::new("open")
                .args(["-a", "Terminal", wt.path.to_str().unwrap()])
                .spawn()?;
        }
        #[cfg(target_os = "linux")]
        {
            // Try common terminal emulators
            let terminals = ["gnome-terminal", "konsole", "xterm"];
            for term in terminals {
                if std::process::Command::new("which")
                    .arg(term)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
                {
                    std::process::Command::new(term)
                        .arg("--working-directory")
                        .arg(&wt.path)
                        .spawn()?;
                    break;
                }
            }
        }
        println!("Opened new terminal in: {}", wt.path.display());
    } else {
        println!("cd {}", wt.path.display());
        println!("\nRun the above command to switch to the worktree.");
    }

    Ok(())
}

fn cmd_clean(repo_root: &PathBuf, dry_run: bool, prune: bool) -> Result<(), GwtError> {
    info!(
        category = "cli",
        command = "clean",
        dry_run,
        prune,
        "Executing clean command"
    );

    let manager = WorktreeManager::new(repo_root)?;

    let orphans = manager.detect_orphans();

    if orphans.is_empty() {
        println!("No orphaned worktrees found.");
        return Ok(());
    }

    for orphan in &orphans {
        println!(
            "{}: {} ({})",
            if dry_run { "Would remove" } else { "Removing" },
            orphan.path.display(),
            orphan.reason
        );

        if !dry_run {
            // Remove orphan (just metadata, path is already gone)
            manager.prune()?;
        }
    }

    if prune && !dry_run {
        manager.prune()?;
        println!("Pruned git worktree metadata.");
    }

    Ok(())
}

fn cmd_logs(repo_root: &Path, settings: &Settings, limit: usize) -> Result<(), GwtError> {
    let log_dir = settings.log_dir(repo_root);
    let reader = gwt_core::logging::LogReader::new(&log_dir);

    let entries = reader.read_latest(limit)?;

    if entries.is_empty() {
        println!("No log entries found.");
        return Ok(());
    }

    for entry in entries {
        println!("{} [{}] {}", entry.timestamp, entry.level, entry.message());
    }

    Ok(())
}

fn cmd_serve(port: u16, address: &str) -> Result<(), GwtError> {
    println!("Starting web server on {}:{}...", address, port);
    // TODO: Start web server (Phase 4)
    println!("Web server is not yet implemented.");
    Ok(())
}

fn cmd_init(repo_root: &Path, force: bool) -> Result<(), GwtError> {
    let config_path = repo_root.join(".gwt.toml");

    if config_path.exists() && !force {
        println!("Configuration already exists at: {}", config_path.display());
        println!("Use --force to overwrite.");
        return Ok(());
    }

    Settings::create_default(&config_path)?;
    println!("Created configuration at: {}", config_path.display());

    Ok(())
}

fn cmd_lock(repo_root: &PathBuf, target: &str, reason: Option<&str>) -> Result<(), GwtError> {
    info!(
        category = "cli",
        command = "lock",
        target,
        reason = reason.unwrap_or("none"),
        "Executing lock command"
    );

    let manager = WorktreeManager::new(repo_root)?;

    let wt = manager
        .get_by_branch(target)?
        .ok_or_else(|| GwtError::WorktreeNotFound {
            path: PathBuf::from(target),
        })?;

    manager.lock(&wt.path, reason)?;
    println!("Locked worktree: {}", wt.path.display());

    Ok(())
}

fn cmd_unlock(repo_root: &PathBuf, target: &str) -> Result<(), GwtError> {
    info!(
        category = "cli",
        command = "unlock",
        target,
        "Executing unlock command"
    );

    let manager = WorktreeManager::new(repo_root)?;

    let wt = manager
        .get_by_branch(target)?
        .ok_or_else(|| GwtError::WorktreeNotFound {
            path: PathBuf::from(target),
        })?;

    manager.unlock(&wt.path)?;
    println!("Unlocked worktree: {}", wt.path.display());

    Ok(())
}

fn cmd_repair(repo_root: &PathBuf, target: Option<&str>) -> Result<(), GwtError> {
    info!(
        category = "cli",
        command = "repair",
        target = target.unwrap_or("all"),
        "Executing repair command"
    );

    let manager = WorktreeManager::new(repo_root)?;

    if let Some(target) = target {
        let wt = manager
            .get_by_branch(target)?
            .ok_or_else(|| GwtError::WorktreeNotFound {
                path: PathBuf::from(target),
            })?;
        manager.repair_path(&wt.path)?;
        println!("Repaired worktree: {}", wt.path.display());
    } else {
        manager.repair()?;
        println!("Repaired all worktrees.");
    }

    Ok(())
}

fn check_git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok()
}

/// Detect package manager from lock files (FR-040a)
fn detect_package_manager(worktree_path: &Path) -> Option<&'static str> {
    // Check lock files in order of preference
    if worktree_path.join("bun.lockb").exists() || worktree_path.join("bun.lock").exists() {
        Some("bun")
    } else if worktree_path.join("pnpm-lock.yaml").exists() {
        Some("pnpm")
    } else if worktree_path.join("yarn.lock").exists() {
        Some("yarn")
    } else if worktree_path.join("package-lock.json").exists() {
        Some("npm")
    } else if worktree_path.join("package.json").exists() {
        // Default to npm if package.json exists but no lock file
        Some("npm")
    } else {
        None
    }
}

/// Install dependencies in worktree (FR-040a, FR-040b)
///
/// FR-040a: Display package manager output directly to stdout/stderr
/// FR-040b: Do NOT use spinner during installation (output would conflict)
fn install_dependencies(worktree_path: &Path) -> Result<(), GwtError> {
    // Check if package.json exists
    if !worktree_path.join("package.json").exists() {
        return Ok(());
    }

    // Check if node_modules already exists (skip if already installed)
    if worktree_path.join("node_modules").exists() {
        return Ok(());
    }

    // Detect package manager
    let pm = match detect_package_manager(worktree_path) {
        Some(pm) => pm,
        None => return Ok(()),
    };

    println!("Installing dependencies with {}...", pm);
    println!();

    // FR-040a: Run package manager with inherited stdout/stderr (no capture)
    // FR-040b: No spinner - just let output flow directly
    let status = Command::new(pm)
        .arg("install")
        .current_dir(worktree_path)
        .status()
        .map_err(|e| GwtError::AgentLaunchFailed {
            name: pm.to_string(),
            reason: format!("Failed to run '{}': {}", pm, e),
        })?;

    if !status.success() {
        eprintln!();
        eprintln!("Warning: {} install exited with status: {}", pm, status);
        // Don't fail - let the user decide whether to continue
    } else {
        println!();
        println!("Dependencies installed successfully.");
    }

    println!();
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentExitKind {
    Success,
    Interrupted,
}

/// Launch a coding agent after TUI exits
///
/// Version selection behavior (FR-066, FR-067, FR-068):
/// - "installed": Use local command if available, fallback to bunx @package@latest
/// - "latest": Use bunx @package@latest
/// - specific version: Use bunx @package@X.Y.Z
fn launch_coding_agent(config: AgentLaunchConfig) -> Result<AgentExitKind, GwtError> {
    // FR-040a/FR-040b: Install dependencies before launching agent
    install_dependencies(&config.worktree_path)?;
    let started_at = Instant::now();

    let cmd_name = config.agent.command_name();
    let npm_package = config.agent.npm_package();

    // Determine execution method based on version selection
    let (executable, base_args, using_local) = if config.version == "installed" {
        // FR-066: Try local command first
        match which::which(cmd_name) {
            Ok(path) => (path.to_string_lossy().to_string(), vec![], true),
            Err(_) => {
                // FR-019: Fallback to bunx @package@latest if local not found
                eprintln!("Note: Local '{}' not found, using bunx fallback", cmd_name);
                let (exe, args) = get_bunx_command(npm_package, "latest");
                (exe, args, false)
            }
        }
    } else if config.version == "latest" {
        // FR-067: Use bunx @package@latest
        let (exe, args) = get_bunx_command(npm_package, "latest");
        (exe, args, false)
    } else {
        // FR-068: Use bunx @package@X.Y.Z for specific version
        let (exe, args) = get_bunx_command(npm_package, &config.version);
        (exe, args, false)
    };

    let package_spec = base_args.first().cloned();

    // Build agent-specific arguments
    let agent_args = build_agent_args(&config);
    let mut command_args = base_args;
    command_args.extend(agent_args.clone());

    let selected_version = if config.version == "installed" && !using_local {
        "latest".to_string()
    } else {
        config.version.clone()
    };
    let version_label = if selected_version == "installed" {
        "installed".to_string()
    } else {
        format!("@{}", selected_version)
    };

    let execution_method = if selected_version == "installed" && using_local {
        ExecutionMethod::Installed {
            command: cmd_name.to_string(),
        }
    } else {
        let label = runner_label(&executable).unwrap_or_else(|| "bunx".to_string());
        let package_spec = package_spec.unwrap_or_else(|| {
            if selected_version == "latest" {
                format!("{}@latest", npm_package)
            } else {
                format!("{}@{}", npm_package, selected_version)
            }
        });
        ExecutionMethod::Runner {
            label,
            package_spec,
        }
    };

    let log_lines = build_launch_log_lines(&config, &agent_args, &version_label, &execution_method);
    for line in log_lines {
        println!("{}", line);
    }
    println!();

    // FR-069, FR-042: Save session entry before launching agent
    let session_entry = ToolSessionEntry {
        branch: config.branch_name.clone(),
        worktree_path: Some(config.worktree_path.to_string_lossy().to_string()),
        tool_id: config.agent.id().to_string(),
        tool_label: config.agent.label().to_string(),
        session_id: None, // Will be updated if agent returns session ID
        mode: Some(config.execution_mode.label().to_string()),
        model: config.model.clone(),
        reasoning_level: config.reasoning_level.map(|r| r.label().to_string()),
        skip_permissions: Some(config.skip_permissions),
        tool_version: Some(selected_version.clone()),
        timestamp: Utc::now().timestamp_millis(),
    };
    if let Err(e) = save_session_entry(&config.worktree_path, session_entry) {
        eprintln!("Warning: Failed to save session: {}", e);
    }

    // Spawn the agent process (FR-043: allows periodic timestamp updates)
    let mut command = Command::new(&executable);
    command
        .args(&command_args)
        .current_dir(&config.worktree_path);
    for (key, value) in &config.env {
        command.env(key, value);
    }
    if config.skip_permissions && config.agent == CodingAgent::ClaudeCode {
        command.env("IS_SANDBOX", "1");
    }

    let mut child = command.spawn().map_err(|e| GwtError::AgentLaunchFailed {
        name: cmd_name.to_string(),
        reason: format!("Failed to execute '{}': {}", executable, e),
    })?;

    // FR-043: Start background thread to update timestamp every 30 seconds
    let running = Arc::new(AtomicBool::new(true));
    let running_clone = Arc::clone(&running);
    let worktree_path = config.worktree_path.clone();
    let branch_name = config.branch_name.clone();
    let agent_id = config.agent.id().to_string();
    let agent_label = config.agent.label().to_string();
    let version = selected_version.clone();
    let model = config.model.clone();
    let mode = config.execution_mode.label().to_string();
    let reasoning_level = config.reasoning_level.map(|r| r.label().to_string());
    let skip_permissions = config.skip_permissions;

    let updater_thread = thread::spawn(move || {
        while running_clone.load(Ordering::Relaxed) {
            // Wait 30 seconds before updating
            thread::sleep(Duration::from_secs(30));

            // Check if still running before updating
            if !running_clone.load(Ordering::Relaxed) {
                break;
            }

            // Update timestamp (FR-043)
            let entry = ToolSessionEntry {
                branch: branch_name.clone(),
                worktree_path: Some(worktree_path.to_string_lossy().to_string()),
                tool_id: agent_id.clone(),
                tool_label: agent_label.clone(),
                session_id: None,
                mode: Some(mode.clone()),
                model: model.clone(),
                reasoning_level: reasoning_level.clone(),
                skip_permissions: Some(skip_permissions),
                tool_version: Some(version.clone()),
                timestamp: Utc::now().timestamp_millis(),
            };
            let _ = save_session_entry(&worktree_path, entry);
        }
    });

    // Wait for the agent process to finish
    let status = child.wait().map_err(|e| GwtError::AgentLaunchFailed {
        name: cmd_name.to_string(),
        reason: format!("Failed to wait for '{}': {}", executable, e),
    })?;

    // Signal the updater thread to stop and wait for it
    running.store(false, Ordering::Relaxed);
    let _ = updater_thread.join();

    if let Some(session_id) = detect_agent_session_id(&config) {
        let entry = ToolSessionEntry {
            branch: config.branch_name.clone(),
            worktree_path: Some(config.worktree_path.to_string_lossy().to_string()),
            tool_id: config.agent.id().to_string(),
            tool_label: config.agent.label().to_string(),
            session_id: Some(session_id),
            mode: Some(config.execution_mode.label().to_string()),
            model: config.model.clone(),
            reasoning_level: config.reasoning_level.map(|r| r.label().to_string()),
            skip_permissions: Some(config.skip_permissions),
            tool_version: Some(selected_version.clone()),
            timestamp: Utc::now().timestamp_millis(),
        };
        if let Err(e) = save_session_entry(&config.worktree_path, entry) {
            eprintln!("Warning: Failed to save session: {}", e);
        }
    }

    let duration_ms = started_at.elapsed().as_millis();
    let exit_info = classify_exit_status(status);
    match exit_info {
        ExitClassification::Success => {
            info!(
                agent_id = config.agent.id(),
                version = selected_version,
                duration_ms = duration_ms as u64,
                "Agent exited successfully"
            );
            Ok(AgentExitKind::Success)
        }
        ExitClassification::Interrupted => {
            warn!(
                agent_id = config.agent.id(),
                version = selected_version,
                duration_ms = duration_ms as u64,
                "Agent session interrupted"
            );
            Ok(AgentExitKind::Interrupted)
        }
        ExitClassification::Failure { code, signal } => {
            error!(
                agent_id = config.agent.id(),
                version = selected_version,
                exit_code = code,
                signal = signal,
                duration_ms = duration_ms as u64,
                "Agent exited with failure"
            );
            let reason = if let Some(signal) = signal {
                format!("Terminated by signal {}", signal)
            } else if let Some(code) = code {
                format_exit_code(code)
            } else {
                "Exited with unknown status".to_string()
            };
            Err(GwtError::AgentLaunchFailed {
                name: cmd_name.to_string(),
                reason,
            })
        }
    }
}

/// Get bunx command and base args for npm package execution
fn get_bunx_command(npm_package: &str, version: &str) -> (String, Vec<String>) {
    // Try bunx first, then npx as fallback
    let bunx_path = which::which("bunx")
        .or_else(|_| which::which("npx"))
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "bunx".to_string());

    let package_spec = if version == "latest" {
        format!("{}@latest", npm_package)
    } else {
        format!("{}@{}", npm_package, version)
    };

    (bunx_path, vec![package_spec])
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExecutionMethod {
    Installed { command: String },
    Runner { label: String, package_spec: String },
}

fn execution_mode_label(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::Normal => "Start new session",
        ExecutionMode::Continue => "Continue session",
        ExecutionMode::Resume => "Resume session",
    }
}

fn extract_codex_model_reasoning(args: &[String]) -> (Option<String>, Option<String>) {
    let mut model = None;
    let mut reasoning = None;
    for arg in args {
        if let Some(value) = arg.strip_prefix("--model=") {
            model = Some(value.to_string());
        }
        if let Some(value) = arg.strip_prefix("model_reasoning_effort=") {
            reasoning = Some(value.to_string());
        }
    }
    (model, reasoning)
}

fn build_launch_log_lines(
    config: &AgentLaunchConfig,
    agent_args: &[String],
    version_label: &str,
    execution_method: &ExecutionMethod,
) -> Vec<String> {
    let mut lines = Vec::new();
    lines.push(format!("Launching {}...", config.agent.label()));
    lines.push(format!(
        "Working directory: {}",
        config.worktree_path.display()
    ));

    match config.agent {
        CodingAgent::CodexCli => {
            let (model, reasoning) = extract_codex_model_reasoning(agent_args);
            if let Some(model) = model {
                lines.push(format!("Model: {}", model));
            }
            if let Some(reasoning) = reasoning {
                lines.push(format!("Reasoning: {}", reasoning));
            }
        }
        _ => {
            if let Some(model) = config.model.as_ref().filter(|m| !m.is_empty()) {
                lines.push(format!("Model: {}", model));
            }
        }
    }

    lines.push(format!(
        "Mode: {}",
        execution_mode_label(config.execution_mode)
    ));
    lines.push(format!(
        "Skip permissions: {}",
        if config.skip_permissions {
            "enabled"
        } else {
            "disabled"
        }
    ));

    let args_text = if agent_args.is_empty() {
        "(none)".to_string()
    } else {
        agent_args.join(" ")
    };
    lines.push(format!("Args: {}", args_text));
    lines.push(format!("Version: {}", version_label));

    match execution_method {
        ExecutionMethod::Installed { command } => {
            lines.push(format!("Using locally installed {}", command));
        }
        ExecutionMethod::Runner {
            label,
            package_spec,
        } => {
            lines.push(format!("Using {} {}", label, package_spec));
        }
    }

    lines
}

fn runner_label(executable: &str) -> Option<String> {
    std::path::Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

fn encode_claude_project_path(path: &Path) -> String {
    path.to_string_lossy()
        .chars()
        .map(|ch| match ch {
            '/' | '\\' | '.' | ':' => '-',
            _ => ch,
        })
        .collect()
}

fn parse_generic_session_id(path: &Path) -> Option<String> {
    let content = fs::read_to_string(path).ok()?;
    let value: serde_json::Value = serde_json::from_str(&content).ok()?;
    let keys = [
        "session_id",
        "sessionId",
        "id",
        "chat_id",
        "chatId",
        "conversation_id",
        "conversationId",
    ];
    for key in keys {
        if let Some(val) = value.get(key) {
            if let Some(text) = val.as_str() {
                return Some(text.to_string());
            }
            if let Some(num) = val.as_i64() {
                return Some(num.to_string());
            }
        }
    }
    None
}

fn parse_codex_session_meta(path: &Path) -> Option<(String, Option<String>)> {
    let file = fs::File::open(path).ok()?;
    let reader = std::io::BufReader::new(file);
    for line in reader.lines().take(5) {
        let line = line.ok()?;
        if line.trim().is_empty() {
            continue;
        }
        let value: serde_json::Value = serde_json::from_str(&line).ok()?;
        let payload = value.get("payload")?;
        let id = payload.get("id")?.as_str()?.to_string();
        let cwd = payload
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        return Some((id, cwd));
    }
    None
}

fn detect_codex_session_id_at(home: &Path, worktree_path: &Path) -> Option<String> {
    let root = home.join(".codex").join("sessions");
    if !root.exists() {
        return None;
    }
    let target_str = worktree_path.to_string_lossy().to_string();
    let target_canon = fs::canonicalize(worktree_path).ok();
    let mut latest_match: Option<(std::time::SystemTime, String)> = None;
    let mut latest_any: Option<(std::time::SystemTime, String)> = None;
    let mut stack = vec![root];

    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let modified = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let Some((id, cwd)) = parse_codex_session_meta(&path) else {
                continue;
            };
            let should_update_any = latest_any
                .as_ref()
                .map(|(time, _)| modified > *time)
                .unwrap_or(true);
            if should_update_any {
                latest_any = Some((modified, id.clone()));
            }

            if let Some(cwd) = cwd {
                let matches_target = if cwd == target_str {
                    true
                } else {
                    let cwd_path = PathBuf::from(&cwd);
                    match (fs::canonicalize(&cwd_path).ok(), target_canon.as_ref()) {
                        (Some(cwd_canon), Some(target_canon)) => cwd_canon == *target_canon,
                        _ => false,
                    }
                };
                if matches_target {
                    let should_update_match = latest_match
                        .as_ref()
                        .map(|(time, _)| modified > *time)
                        .unwrap_or(true);
                    if should_update_match {
                        latest_match = Some((modified, id));
                    }
                }
            }
        }
    }

    latest_match
        .or(latest_any)
        .map(|(_, session_id)| session_id)
}

fn detect_claude_session_id_at(home: &Path, worktree_path: &Path) -> Option<String> {
    let project_dir = home
        .join(".claude")
        .join("projects")
        .join(encode_claude_project_path(worktree_path));
    if !project_dir.exists() {
        return None;
    }
    let entries = fs::read_dir(&project_dir).ok()?;
    let mut latest: Option<(std::time::SystemTime, String)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        let modified = metadata
            .modified()
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let stem = path.file_stem().and_then(|s| s.to_str())?.to_string();
        let should_update = latest
            .as_ref()
            .map(|(time, _)| modified > *time)
            .unwrap_or(true);
        if should_update {
            latest = Some((modified, stem));
        }
    }
    latest.map(|(_, session_id)| session_id)
}

fn detect_gemini_session_id_at(home: &Path) -> Option<String> {
    let root = home.join(".gemini").join("tmp");
    if !root.exists() {
        return None;
    }
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let path_text = path.to_string_lossy();
            if !path_text.contains("/chats/") {
                continue;
            }
            let modified = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let should_update = latest
                .as_ref()
                .map(|(time, _)| modified > *time)
                .unwrap_or(true);
            if should_update {
                latest = Some((modified, path));
            }
        }
    }
    let path = latest.map(|(_, path)| path)?;
    parse_generic_session_id(&path).or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    })
}

fn detect_opencode_session_id_at(home: &Path) -> Option<String> {
    let root = home.join(".local").join("share").join("opencode");
    if !root.exists() {
        return None;
    }
    let mut latest: Option<(std::time::SystemTime, PathBuf)> = None;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(_) => continue,
            };
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            let ext = path.extension().and_then(|ext| ext.to_str());
            if ext != Some("json") && ext != Some("jsonl") {
                continue;
            }
            let modified = metadata
                .modified()
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            let should_update = latest
                .as_ref()
                .map(|(time, _)| modified > *time)
                .unwrap_or(true);
            if should_update {
                latest = Some((modified, path));
            }
        }
    }
    let path = latest.map(|(_, path)| path)?;
    parse_generic_session_id(&path).or_else(|| {
        path.file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
    })
}

fn detect_agent_session_id(config: &AgentLaunchConfig) -> Option<String> {
    let home = home_dir()?;
    match config.agent {
        CodingAgent::CodexCli => detect_codex_session_id_at(&home, &config.worktree_path),
        CodingAgent::ClaudeCode => detect_claude_session_id_at(&home, &config.worktree_path),
        CodingAgent::GeminiCli => detect_gemini_session_id_at(&home),
        CodingAgent::OpenCode => detect_opencode_session_id_at(&home),
    }
}

/// Build agent-specific command line arguments
fn build_agent_args(config: &AgentLaunchConfig) -> Vec<String> {
    let mut args = Vec::new();

    match config.agent {
        CodingAgent::ClaudeCode => {
            // Model selection
            if let Some(model) = &config.model {
                if !model.is_empty() {
                    args.push("--model".to_string());
                    args.push(model.clone());
                }
            }

            // Execution mode (FR-102)
            match config.execution_mode {
                ExecutionMode::Continue | ExecutionMode::Resume => {
                    if let Some(session_id) = &config.session_id {
                        args.push("--resume".to_string());
                        args.push(session_id.clone());
                    } else if matches!(config.execution_mode, ExecutionMode::Continue) {
                        args.push("-c".to_string());
                    } else {
                        args.push("-r".to_string());
                    }
                }
                ExecutionMode::Normal => {}
            }

            // Skip permissions
            if config.skip_permissions {
                args.push("--dangerously-skip-permissions".to_string());
            }
        }
        CodingAgent::CodexCli => {
            // Execution mode - resume subcommand must come first
            match config.execution_mode {
                ExecutionMode::Continue | ExecutionMode::Resume => {
                    args.push("resume".to_string());
                    if let Some(session_id) = &config.session_id {
                        args.push(session_id.clone());
                    } else if matches!(config.execution_mode, ExecutionMode::Continue) {
                        args.push("--last".to_string());
                    }
                }
                ExecutionMode::Normal => {}
            }

            // Skip permissions (Codex uses versioned flag)
            let flag_version = resolve_codex_flag_version(config);
            let skip_flag = if config.skip_permissions {
                Some(codex_skip_permissions_flag(flag_version.as_deref()))
            } else {
                None
            };
            let bypass_sandbox = matches!(
                skip_flag,
                Some("--dangerously-bypass-approvals-and-sandbox")
            );

            let reasoning_override = config.reasoning_level.map(|r| r.label());
            let skills_flag_version = flag_version;
            args.extend(codex_default_args(
                config.model.as_deref(),
                reasoning_override,
                skills_flag_version.as_deref(),
                bypass_sandbox,
            ));

            if let Some(flag) = skip_flag {
                args.push(flag.to_string());
            }
        }
        CodingAgent::GeminiCli => {
            // Model selection (Gemini uses -m or --model)
            if let Some(model) = &config.model {
                if !model.is_empty() {
                    args.push("-m".to_string());
                    args.push(model.clone());
                }
            }

            // Execution mode
            match config.execution_mode {
                ExecutionMode::Continue | ExecutionMode::Resume => {
                    args.push("-r".to_string());
                    if let Some(session_id) = &config.session_id {
                        args.push(session_id.clone());
                    } else {
                        args.push("latest".to_string());
                    }
                }
                ExecutionMode::Normal => {}
            }

            // Skip permissions (Gemini uses -y or --yolo)
            if config.skip_permissions {
                args.push("-y".to_string());
            }
        }
        CodingAgent::OpenCode => {
            // Model selection
            if let Some(model) = &config.model {
                if !model.is_empty() {
                    args.push("--model".to_string());
                    args.push(model.clone());
                }
            }

            // Execution mode
            match config.execution_mode {
                ExecutionMode::Continue => args.push("-c".to_string()),
                ExecutionMode::Resume => {
                    if let Some(session_id) = &config.session_id {
                        args.push("-s".to_string());
                        args.push(session_id.clone());
                    }
                }
                ExecutionMode::Normal => {}
            }
        }
    }

    args
}

fn resolve_codex_flag_version(config: &AgentLaunchConfig) -> Option<String> {
    match config.version.as_str() {
        "installed" => get_command_version("codex", "--version"),
        "latest" => None,
        other => Some(other.to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExitClassification {
    Success,
    Interrupted,
    Failure {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

fn classify_exit(code: Option<i32>, signal: Option<i32>) -> ExitClassification {
    if code == Some(0) && signal.is_none() {
        return ExitClassification::Success;
    }

    if let Some(sig) = signal {
        if sig == 2 || sig == 15 {
            return ExitClassification::Interrupted;
        }
    }

    if matches!(code, Some(130 | 143)) {
        return ExitClassification::Interrupted;
    }

    ExitClassification::Failure { code, signal }
}

#[cfg(unix)]
fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    status.signal()
}

#[cfg(not(unix))]
fn exit_signal(_status: &std::process::ExitStatus) -> Option<i32> {
    None
}

fn classify_exit_status(status: std::process::ExitStatus) -> ExitClassification {
    classify_exit(status.code(), exit_signal(&status))
}

/// Format exit code with platform-specific explanation
fn format_exit_code(code: i32) -> String {
    // Negative codes on Windows are typically NTSTATUS values
    if code < 0 {
        let ntstatus = code as u32;
        if let Some(desc) = describe_ntstatus(ntstatus) {
            return format!("Exited with status {} (0x{:08X}: {})", code, ntstatus, desc);
        }
        return format!("Exited with status {} (0x{:08X})", code, ntstatus);
    }
    format!("Exited with status {}", code)
}

/// Describe common NTSTATUS codes (Windows-specific error codes)
fn describe_ntstatus(code: u32) -> Option<&'static str> {
    match code {
        0xC0000005 => Some("STATUS_ACCESS_VIOLATION"),
        0xC0000017 => Some("STATUS_NO_MEMORY"),
        0xC000001D => Some("STATUS_ILLEGAL_INSTRUCTION"),
        0xC00000FD => Some("STATUS_STACK_OVERFLOW"),
        0xC0000135 => Some("STATUS_DLL_NOT_FOUND"),
        0xC0000139 => Some("STATUS_ENTRYPOINT_NOT_FOUND"),
        0xC0000142 => Some("STATUS_DLL_INIT_FAILED"),
        0xC0000374 => Some("STATUS_HEAP_CORRUPTION"),
        0xC0000409 => Some("STATUS_STACK_BUFFER_OVERRUN"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::thread::sleep;
    use tempfile::TempDir;

    fn sample_config(agent: CodingAgent) -> AgentLaunchConfig {
        AgentLaunchConfig {
            worktree_path: PathBuf::from("/tmp/worktree"),
            branch_name: "feature/test".to_string(),
            agent,
            model: Some("sonnet".to_string()),
            reasoning_level: None,
            version: "latest".to_string(),
            execution_mode: ExecutionMode::Continue,
            session_id: None,
            skip_permissions: true,
            env: Vec::new(),
        }
    }

    #[test]
    fn test_classify_exit_success() {
        assert!(matches!(
            classify_exit(Some(0), None),
            ExitClassification::Success
        ));
    }

    #[test]
    fn test_classify_exit_interrupted_signal() {
        assert!(matches!(
            classify_exit(None, Some(2)),
            ExitClassification::Interrupted
        ));
    }

    #[test]
    fn test_classify_exit_interrupted_code() {
        assert!(matches!(
            classify_exit(Some(130), None),
            ExitClassification::Interrupted
        ));
    }

    #[test]
    fn test_classify_exit_failure() {
        assert!(matches!(
            classify_exit(Some(1), None),
            ExitClassification::Failure { .. }
        ));
    }

    #[test]
    fn test_build_launch_log_lines_codex() {
        let mut config = sample_config(CodingAgent::CodexCli);
        config.model = None;
        let agent_args = vec![
            "resume".to_string(),
            "--last".to_string(),
            "--model=gpt-5.2-codex".to_string(),
            "-c".to_string(),
            "model_reasoning_effort=high".to_string(),
        ];
        let lines = build_launch_log_lines(
            &config,
            &agent_args,
            "@latest",
            &ExecutionMethod::Runner {
                label: "bunx".to_string(),
                package_spec: "@openai/codex@latest".to_string(),
            },
        );
        assert!(lines.contains(&"Launching Codex CLI...".to_string()));
        assert!(lines.contains(&"Working directory: /tmp/worktree".to_string()));
        assert!(lines.contains(&"Model: gpt-5.2-codex".to_string()));
        assert!(lines.contains(&"Reasoning: high".to_string()));
        assert!(lines.contains(&"Mode: Continue session".to_string()));
        assert!(lines.contains(&"Skip permissions: enabled".to_string()));
        assert!(lines.contains(&"Version: @latest".to_string()));
        assert!(lines.contains(&"Using bunx @openai/codex@latest".to_string()));
        assert!(lines
            .iter()
            .any(|line| line.contains("Args: resume --last --model=gpt-5.2-codex")));
    }

    #[test]
    fn test_build_launch_log_lines_non_codex() {
        let config = sample_config(CodingAgent::ClaudeCode);
        let agent_args = vec![
            "--model".to_string(),
            "sonnet".to_string(),
            "--dangerously-skip-permissions".to_string(),
        ];
        let lines = build_launch_log_lines(
            &config,
            &agent_args,
            "installed",
            &ExecutionMethod::Installed {
                command: "claude".to_string(),
            },
        );
        assert!(lines.contains(&"Model: sonnet".to_string()));
        assert!(!lines.iter().any(|line| line.starts_with("Reasoning: ")));
        assert!(lines.contains(&"Using locally installed claude".to_string()));
    }

    #[test]
    fn test_build_agent_args_codex_skip_flag_last() {
        let mut config = sample_config(CodingAgent::CodexCli);
        config.execution_mode = ExecutionMode::Normal;
        config.model = None;
        let args = build_agent_args(&config);
        assert_eq!(
            args.last().map(String::as_str),
            Some("--dangerously-bypass-approvals-and-sandbox")
        );
        assert!(!args.contains(&"--sandbox".to_string()));
        assert!(!args.contains(&"sandbox_workspace_write.network_access=true".to_string()));
    }

    #[test]
    fn test_build_agent_args_codex_includes_session_id() {
        let mut config = sample_config(CodingAgent::CodexCli);
        config.session_id = Some("sess-123".to_string());
        let args = build_agent_args(&config);
        assert_eq!(args.first(), Some(&"resume".to_string()));
        assert_eq!(args.get(1), Some(&"sess-123".to_string()));
        assert!(!args.contains(&"--last".to_string()));
    }

    #[test]
    fn test_build_agent_args_claude_resume_uses_session_id() {
        let mut config = sample_config(CodingAgent::ClaudeCode);
        config.execution_mode = ExecutionMode::Resume;
        config.session_id = Some("abc123".to_string());
        let args = build_agent_args(&config);
        let resume_pos = args.iter().position(|arg| arg == "--resume").unwrap();
        assert_eq!(args.get(resume_pos + 1), Some(&"abc123".to_string()));
        assert!(!args.contains(&"-r".to_string()));
        assert!(!args.contains(&"-c".to_string()));
    }

    #[test]
    fn test_build_agent_args_gemini_resume_uses_session_id() {
        let mut config = sample_config(CodingAgent::GeminiCli);
        config.execution_mode = ExecutionMode::Resume;
        config.session_id = Some("g-999".to_string());
        let args = build_agent_args(&config);
        let resume_pos = args.iter().position(|arg| arg == "-r").unwrap();
        assert_eq!(args.get(resume_pos + 1), Some(&"g-999".to_string()));
    }

    #[test]
    fn test_build_agent_args_opencode_resume_uses_session_id() {
        let mut config = sample_config(CodingAgent::OpenCode);
        config.execution_mode = ExecutionMode::Resume;
        config.session_id = Some("oc-1".to_string());
        let args = build_agent_args(&config);
        let resume_pos = args.iter().position(|arg| arg == "-s").unwrap();
        assert_eq!(args.get(resume_pos + 1), Some(&"oc-1".to_string()));
    }

    #[test]
    fn test_detect_codex_session_id_prefers_matching_cwd() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let sessions_dir = home.join(".codex").join("sessions").join("2026");
        fs::create_dir_all(&sessions_dir).unwrap();

        let match_path = sessions_dir.join("match.jsonl");
        fs::write(
            &match_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"match\",\"cwd\":\"/repo/wt\"}}",
        )
        .unwrap();

        sleep(std::time::Duration::from_secs(1));

        let other_path = sessions_dir.join("other.jsonl");
        fs::write(
            &other_path,
            "{\"type\":\"session_meta\",\"payload\":{\"id\":\"other\",\"cwd\":\"/repo/other\"}}",
        )
        .unwrap();

        let id = detect_codex_session_id_at(home, Path::new("/repo/wt"));
        assert_eq!(id.as_deref(), Some("match"));
    }

    #[test]
    fn test_detect_claude_session_id_uses_latest_file() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let project_dir = home
            .join(".claude")
            .join("projects")
            .join(encode_claude_project_path(Path::new("/repo/wt")));
        fs::create_dir_all(&project_dir).unwrap();

        fs::write(project_dir.join("first.jsonl"), "{}").unwrap();
        sleep(std::time::Duration::from_secs(1));
        fs::write(project_dir.join("second.jsonl"), "{}").unwrap();

        let id = detect_claude_session_id_at(home, Path::new("/repo/wt"));
        assert_eq!(id.as_deref(), Some("second"));
    }

    #[test]
    fn test_format_exit_code_normal() {
        assert_eq!(format_exit_code(0), "Exited with status 0");
        assert_eq!(format_exit_code(1), "Exited with status 1");
        assert_eq!(format_exit_code(127), "Exited with status 127");
    }

    #[test]
    fn test_format_exit_code_negative_known() {
        // 0xC0000409 as i32 = -1073740791 (STATUS_STACK_BUFFER_OVERRUN)
        let result = format_exit_code(-1073740791);
        assert!(result.contains("-1073740791"));
        assert!(result.contains("0xC0000409"));
        assert!(result.contains("STATUS_STACK_BUFFER_OVERRUN"));

        // 0xC0000374 as i32 = -1073740940 (STATUS_HEAP_CORRUPTION)
        let result = format_exit_code(-1073740940);
        assert!(result.contains("-1073740940"));
        assert!(result.contains("0xC0000374"));
        assert!(result.contains("STATUS_HEAP_CORRUPTION"));
    }

    #[test]
    fn test_format_exit_code_negative_unknown() {
        // Unknown NTSTATUS (0xFFFFFFFF = -1)
        let result = format_exit_code(-1);
        assert!(result.contains("-1"));
        assert!(result.contains("0x"));
        // No STATUS_ description for unknown code
        assert!(!result.contains("STATUS_"));
    }

    #[test]
    fn test_describe_ntstatus() {
        assert_eq!(
            describe_ntstatus(0xC0000374),
            Some("STATUS_HEAP_CORRUPTION")
        );
        assert_eq!(
            describe_ntstatus(0xC0000005),
            Some("STATUS_ACCESS_VIOLATION")
        );
        assert_eq!(describe_ntstatus(0x12345678), None);
    }
}
