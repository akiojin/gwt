//! CLI command handlers

use crate::cli::{Commands, HookAction, OutputFormat};
use gwt_core::config::{Session, Settings};
use gwt_core::error::GwtError;
use gwt_core::git::Branch;
use gwt_core::logging::{LogEntry, LogReader};
use gwt_core::worktree::WorktreeManager;
use gwt_web::ServerConfig;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::runtime::Builder;
use tracing::{debug, error, info};

pub fn handle_command(
    cmd: Commands,
    repo_root: &PathBuf,
    settings: &Settings,
) -> Result<(), GwtError> {
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
        Commands::Logs { limit, follow } => cmd_logs(repo_root, settings, limit, follow),
        Commands::Serve { port, address } => cmd_serve(repo_root, settings, port, address.as_deref()),
        Commands::Init { url, force, full } => cmd_init(repo_root, url.as_deref(), force, full),
        Commands::Lock { target, reason } => cmd_lock(repo_root, &target, reason.as_deref()),
        Commands::Unlock { target } => cmd_unlock(repo_root, &target),
        Commands::Hook { action } => cmd_hook(action),
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
            let output =
                serde_json::to_string_pretty(&json).map_err(|e| GwtError::Internal(e.to_string()))?;
            println!("{}", output);
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
    let wt = manager.get_by_branch(target)?.or_else(|| {
        let path = PathBuf::from(target);
        manager.get_by_path(&path).ok().flatten()
    });

    if delete_branch {
        if let Some(wt) = wt {
            if let Some(branch) = wt.branch.as_deref() {
                manager.cleanup_branch(branch, force, force)?;
                println!("Removed worktree and branch: {}", branch);
            } else {
                manager.remove(&wt.path, force)?;
                println!("Removed worktree: {}", wt.path.display());
            }
            return Ok(());
        }

        if Branch::exists(repo_root, target)? {
            manager.cleanup_branch(target, force, force)?;
            println!("Removed branch: {}", target);
            return Ok(());
        }

        return Err(GwtError::BranchNotFound {
            name: target.to_string(),
        });
    }

    let wt = wt.ok_or_else(|| GwtError::WorktreeNotFound {
        path: PathBuf::from(target),
    })?;
    manager.remove(&wt.path, force)?;
    println!("Removed worktree: {}", wt.path.display());

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
            let path_str = wt.path.to_str().ok_or_else(|| {
                GwtError::Internal("Worktree path is not valid UTF-8".to_string())
            })?;
            std::process::Command::new("open")
                .args(["-a", "Terminal", path_str])
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

fn cmd_logs(
    repo_root: &Path,
    settings: &Settings,
    limit: usize,
    follow: bool,
) -> Result<(), GwtError> {
    let log_dir = settings.log_dir(repo_root);
    let reader = LogReader::new(&log_dir);

    if !follow {
        let entries = reader.read_latest(limit)?;

        if entries.is_empty() {
            println!("No log entries found.");
            return Ok(());
        }

        for entry in entries {
            println!("{} [{}] {}", entry.timestamp, entry.level, entry.message());
        }

        return Ok(());
    }

    let files = reader.list_files()?;
    let Some(latest) = files.first() else {
        println!("No log entries found.");
        return Ok(());
    };

    let total_lines = count_log_lines(latest)?;
    let start = total_lines.saturating_sub(limit);
    let (entries, _) = LogReader::read_entries(latest, start, limit)?;

    if entries.is_empty() {
        println!("No log entries found.");
    } else {
        for entry in &entries {
            println!("{} [{}] {}", entry.timestamp, entry.level, entry.message());
        }
    }

    let mut offset = start + entries.len();
    loop {
        std::thread::sleep(Duration::from_millis(500));
        let new_entries = read_new_entries(latest, offset)?;
        if new_entries.is_empty() {
            continue;
        }

        for entry in &new_entries {
            println!("{} [{}] {}", entry.timestamp, entry.level, entry.message());
        }
        offset += new_entries.len();
    }
}

fn count_log_lines(path: &Path) -> Result<usize, GwtError> {
    use std::io::{BufRead, BufReader};

    let file = std::fs::File::open(path)?;
    let reader = BufReader::new(file);
    Ok(reader.lines().count())
}

fn read_new_entries(path: &Path, offset: usize) -> Result<Vec<LogEntry>, GwtError> {
    let (entries, _) = LogReader::read_entries(path, offset, usize::MAX)?;
    Ok(entries)
}

fn build_server_config(
    repo_root: &Path,
    settings: &Settings,
    port: Option<u16>,
    address: Option<&str>,
) -> ServerConfig {
    let resolved_port = port.unwrap_or(settings.web.port);
    let resolved_address = address
        .map(str::to_string)
        .unwrap_or_else(|| settings.web.address.clone());

    ServerConfig::new(resolved_port)
        .with_address(resolved_address)
        .with_repo_path(repo_root.to_path_buf())
        .with_cors(settings.web.cors)
}

fn cmd_serve(
    repo_root: &Path,
    settings: &Settings,
    port: Option<u16>,
    address: Option<&str>,
) -> Result<(), GwtError> {
    let config = build_server_config(repo_root, settings, port, address);
    println!(
        "Starting web server on {}:{}...",
        config.address, config.port
    );
    let runtime = Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| GwtError::Internal(e.to_string()))?;

    let address_label = format!("{}:{}", config.address, config.port);
    let result = runtime.block_on(async { gwt_web::serve_with_config(config).await });
    match result {
        Ok(()) => Ok(()),
        Err(err) => {
            error!(category = "server", error = %err, "Web server failed to start");
            Err(GwtError::ServerBindFailed {
                address: address_label,
            })
        }
    }
}

/// Initialize gwt: clone a bare repository or create config (SPEC-a70a1ece T312-T313)
fn cmd_init(repo_root: &Path, url: Option<&str>, force: bool, full: bool) -> Result<(), GwtError> {
    // If URL is provided, clone as bare repository
    if let Some(url) = url {
        use gwt_core::git::{clone_bare, CloneConfig};

        info!(
            category = "cli",
            command = "init",
            url,
            full,
            "Cloning bare repository"
        );

        // T313: Default to shallow clone (--depth=1) unless --full is specified
        let config = if full {
            CloneConfig::bare(url, repo_root)
        } else {
            CloneConfig::bare_shallow(url, repo_root, 1)
        };

        let clone_type = if full { "full" } else { "shallow (--depth=1)" };
        println!("Cloning {} as {} bare repository...", url, clone_type);

        match clone_bare(&config) {
            Ok(path) => {
                println!("Successfully cloned to: {}", path.display());
                println!("\nNext steps:");
                println!("  cd {}", path.display());
                println!("  gwt           # Open TUI to create worktree");
                Ok(())
            }
            Err(e) => {
                error!(category = "cli", error = %e, "Failed to clone repository");
                Err(e)
            }
        }
    } else {
        // Original behavior: create config file
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

/// Handle Claude Code hook subcommands (SPEC-861d8cdf FR-101/T-101/T-102)
fn cmd_hook(action: HookAction) -> Result<(), GwtError> {
    use gwt_core::config::{
        get_claude_settings_path, is_gwt_hooks_registered, register_gwt_hooks, unregister_gwt_hooks,
    };

    match action {
        HookAction::Event { name } => handle_hook_event(&name),
        HookAction::EventAlias(args) => {
            let name = args
                .first()
                .ok_or_else(|| GwtError::Internal("Missing hook event name.".to_string()))?;
            if args.len() > 1 {
                return Err(GwtError::Internal(format!(
                    "Unexpected hook arguments: {}",
                    args.join(" ")
                )));
            }
            handle_hook_event(name)
        }
        HookAction::Setup => {
            let settings_path =
                get_claude_settings_path().ok_or_else(|| GwtError::ConfigNotFound {
                    path: PathBuf::from("~/.claude/settings.json"),
                })?;

            if is_gwt_hooks_registered(&settings_path) {
                println!("gwt hooks are already registered in Claude Code settings.");
                return Ok(());
            }

            register_gwt_hooks(&settings_path)?;
            println!("Successfully registered gwt hooks in Claude Code settings.");
            println!("Path: {}", settings_path.display());
            Ok(())
        }
        HookAction::Uninstall => {
            let settings_path =
                get_claude_settings_path().ok_or_else(|| GwtError::ConfigNotFound {
                    path: PathBuf::from("~/.claude/settings.json"),
                })?;

            if !is_gwt_hooks_registered(&settings_path) {
                println!("gwt hooks are not registered in Claude Code settings.");
                return Ok(());
            }

            unregister_gwt_hooks(&settings_path)?;
            println!("Successfully removed gwt hooks from Claude Code settings.");
            Ok(())
        }
        HookAction::Status => {
            let settings_path =
                get_claude_settings_path().ok_or_else(|| GwtError::ConfigNotFound {
                    path: PathBuf::from("~/.claude/settings.json"),
                })?;

            if is_gwt_hooks_registered(&settings_path) {
                println!("gwt hooks: registered");
                println!("Path: {}", settings_path.display());
            } else {
                println!("gwt hooks: not registered");
                println!("Run 'gwt hook setup' to enable agent status tracking.");
            }
            Ok(())
        }
    }
}

/// Process a hook event from Claude Code (SPEC-861d8cdf T-101)
/// Called by Claude Code hooks via `gwt hook <name>` (or `gwt hook event <name>`)
fn handle_hook_event(event: &str) -> Result<(), GwtError> {
    use std::io::{self, Read};

    info!(
        category = "cli",
        command = "hook",
        event = event,
        "Executing hook event command"
    );

    // Read JSON payload from stdin
    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    // Parse the JSON payload
    let payload: serde_json::Value = serde_json::from_str(&input).unwrap_or_default();

    // Extract cwd from payload to determine which worktree to update
    let cwd = payload
        .get("cwd")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    debug!(
        category = "hook",
        event = event,
        cwd = %cwd.display(),
        "Processing hook event"
    );

    // Determine the new status based on the event
    let new_status = crate::hook_event_to_status(event, &payload);

    // Load or create session for the worktree
    let session_path = Session::session_path(&cwd);
    let mut session = if session_path.exists() {
        Session::load(&session_path).unwrap_or_else(|_| {
            // Create new session if load fails
            let branch = crate::detect_branch_name(&cwd);
            Session::new(&cwd, &branch)
        })
    } else {
        let branch = crate::detect_branch_name(&cwd);
        Session::new(&cwd, &branch)
    };

    // Update the session status
    session.update_status(new_status);
    session.save(&session_path)?;

    debug!(
        category = "hook",
        event = event,
        status = ?new_status,
        session_path = %session_path.display(),
        "Session status updated"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{build_server_config, count_log_lines, read_new_entries};
    use gwt_core::config::Settings;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_build_server_config_uses_settings_when_cli_missing() {
        let repo = TempDir::new().unwrap();
        let mut settings = Settings::default();
        settings.web.port = 4123;
        settings.web.address = "0.0.0.0".to_string();
        settings.web.cors = false;
        let config = build_server_config(repo.path(), &settings, None, None);

        assert_eq!(config.port, 4123);
        assert_eq!(config.address, "0.0.0.0");
        assert_eq!(config.repo_path, repo.path());
        assert!(!config.cors_enabled);
    }

    #[test]
    fn test_build_server_config_cli_overrides_settings() {
        let repo = TempDir::new().unwrap();
        let mut settings = Settings::default();
        settings.web.port = 3000;
        settings.web.address = "127.0.0.1".to_string();
        settings.web.cors = true;
        let config = build_server_config(repo.path(), &settings, Some(4001), Some("0.0.0.0"));

        assert_eq!(config.port, 4001);
        assert_eq!(config.address, "0.0.0.0");
        assert_eq!(config.repo_path, repo.path());
        assert!(config.cors_enabled);
    }

    #[test]
    fn test_read_new_entries_after_append() {
        let temp = TempDir::new().unwrap();
        let log_file = temp.path().join("gwt.jsonl.2024-01-01");
        let first = r#"{"timestamp":"2024-01-01T00:00:00Z","level":"INFO","fields":{"message":"one"},"target":"gwt"}"#;
        let second = r#"{"timestamp":"2024-01-01T00:00:01Z","level":"INFO","fields":{"message":"two"},"target":"gwt"}"#;
        std::fs::write(&log_file, format!("{}\n{}\n", first, second)).unwrap();

        let total = count_log_lines(&log_file).unwrap();
        assert_eq!(total, 2);

        let entries = read_new_entries(&log_file, 0).unwrap();
        assert_eq!(entries.len(), 2);

        let third = r#"{"timestamp":"2024-01-01T00:00:02Z","level":"INFO","fields":{"message":"three"},"target":"gwt"}"#;
        std::fs::OpenOptions::new()
            .append(true)
            .open(&log_file)
            .unwrap()
            .write_all(format!("{}\n", third).as_bytes())
            .unwrap();

        let new_entries = read_new_entries(&log_file, 2).unwrap();
        assert_eq!(new_entries.len(), 1);
        assert_eq!(new_entries[0].message(), "three");
    }
}
