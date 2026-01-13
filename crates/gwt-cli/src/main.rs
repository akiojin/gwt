//! gwt - Git Worktree Manager CLI

use chrono::Utc;
use clap::Parser;
use gwt_core::config::{save_session_entry, Settings, ToolSessionEntry};
use gwt_core::error::GwtError;
use gwt_core::worktree::WorktreeManager;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

mod cli;
mod tui;

use cli::{Cli, Commands, OutputFormat};
use tui::{AgentLaunchConfig, CodingAgent, ExecutionMode};

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
    let log_config = gwt_core::logging::LogConfig {
        debug: cli.debug || std::env::var("GWT_DEBUG").is_ok(),
        log_dir: settings.log_dir(&repo_root),
        ..Default::default()
    };
    gwt_core::logging::init_logger(&log_config)?;

    match cli.command {
        Some(cmd) => handle_command(cmd, &repo_root, &settings),
        None => {
            // Interactive TUI mode
            if let Ok(manager) = WorktreeManager::new(&repo_root) {
                let _ = manager.auto_cleanup_orphans();
            }
            match tui::run()? {
                Some(launch_config) => launch_coding_agent(launch_config),
                None => Ok(()),
            }
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
        println!("{} [{}] {}", entry.timestamp, entry.level, entry.message);
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

/// Launch a coding agent after TUI exits
///
/// Version selection behavior (FR-066, FR-067, FR-068):
/// - "installed": Use local command if available, fallback to bunx @package@latest
/// - "latest": Use bunx @package@latest
/// - specific version: Use bunx @package@X.Y.Z
fn launch_coding_agent(config: AgentLaunchConfig) -> Result<(), GwtError> {
    // FR-040a/FR-040b: Install dependencies before launching agent
    install_dependencies(&config.worktree_path)?;

    let cmd_name = config.agent.command_name();
    let npm_package = config.agent.npm_package();

    // Determine execution method based on version selection
    let (executable, mut base_args, using_local) = if config.version == "installed" {
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

    // Build agent-specific arguments
    let agent_args = build_agent_args(&config);
    base_args.extend(agent_args);

    // Print launch info (FR-072, FR-073)
    println!(
        "Launching {} in {}",
        config.agent.label(),
        config.worktree_path.display()
    );
    // FR-072: Version format varies by selection type
    if config.version == "installed" {
        println!("Version: installed");
        // FR-073: Only show "Using locally installed" for installed selection
        if using_local {
            println!("Using locally installed");
        }
    } else {
        println!("Version: @{}", config.version);
    }
    println!("Command: {} {}", executable, base_args.join(" "));
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
        tool_version: Some(config.version.clone()),
        timestamp: Utc::now().timestamp_millis(),
    };
    if let Err(e) = save_session_entry(&config.worktree_path, session_entry) {
        eprintln!("Warning: Failed to save session: {}", e);
    }

    // Spawn the agent process (FR-043: allows periodic timestamp updates)
    let mut command = Command::new(&executable);
    command.args(&base_args).current_dir(&config.worktree_path);
    for (key, value) in &config.env {
        command.env(key, value);
    }
    if config.skip_permissions
        && is_root_user()
        && !env_has_key(&config.env, "IS_SANDBOX")
        && config.agent == CodingAgent::ClaudeCode
    {
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
    let version = config.version.clone();
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

    if !status.success() {
        eprintln!("Warning: {} exited with status: {}", cmd_name, status);
    }

    Ok(())
}

fn is_root_user() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::geteuid() == 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

fn env_has_key(env: &[(String, String)], key: &str) -> bool {
    env.iter().any(|(k, _)| k == key)
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
                ExecutionMode::Continue => args.push("-c".to_string()),
                ExecutionMode::Resume => args.push("-r".to_string()),
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
                ExecutionMode::Continue => {
                    args.push("resume".to_string());
                    args.push("--last".to_string());
                }
                ExecutionMode::Resume => {
                    args.push("resume".to_string());
                }
                ExecutionMode::Normal => {}
            }

            // Skip permissions (Codex uses --yolo)
            if config.skip_permissions {
                args.push("--yolo".to_string());
            }

            args.extend(codex_default_args(config.model.as_deref()));
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
                    args.push("latest".to_string());
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
        }
    }

    args
}

fn codex_default_args(model_override: Option<&str>) -> Vec<String> {
    let mut args = Vec::new();
    args.push("--search".to_string());
    if let Some(model) = model_override {
        if !model.is_empty() {
            args.push(format!("--model=\"{}\"", model));
        } else {
            args.push("--model=\"gpt-5-codex\"".to_string());
        }
    } else {
        args.push("--model=\"gpt-5-codex\"".to_string());
    }
    args.push("--sandbox".to_string());
    args.push("workspace-write".to_string());
    args.push("-c".to_string());
    args.push("model_reasoning_effort=\"high\"".to_string());
    args.push("-c".to_string());
    args.push("model_reasoning_summaries=\"detailed\"".to_string());
    args.push("-c".to_string());
    args.push("sandbox_workspace_write.network_access=true".to_string());
    args.push("-c".to_string());
    args.push("shell_environment_policy.inherit=all".to_string());
    args.push("-c".to_string());
    args.push("shell_environment_policy.ignore_default_excludes=true".to_string());
    args.push("-c".to_string());
    args.push("shell_environment_policy.experimental_use_profile=true".to_string());
    args
}
