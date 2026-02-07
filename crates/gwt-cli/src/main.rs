//! gwt - Git Worktree Manager CLI

use chrono::Utc;
use clap::Parser;
use gwt_core::agent::codex::{codex_default_args, codex_skip_permissions_flag};
use gwt_core::agent::get_command_version;
use gwt_core::ai::{
    AgentHistoryStore, AgentType as SessionAgentType, ClaudeSessionParser, CodexSessionParser,
    GeminiSessionParser, OpenCodeSessionParser, SessionParser,
};
use gwt_core::config::{save_session_entry, AgentStatus, Settings, ToolSessionEntry};
use gwt_core::error::GwtError;
use std::fs;
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};
use tracing::{debug, error, info, warn};

mod cli;
mod commands;
mod tui;

use cli::Cli;
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
    match cleanup_startup_logs(&repo_root, &settings) {
        Ok(removed) => {
            if removed > 0 {
                info!(category = "logging", removed, "Removed old log files");
            }
        }
        Err(err) => {
            warn!(category = "logging", error = %err, "Failed to clean up old logs");
        }
    }

    info!(
        repo_root = %repo_root.display(),
        debug = log_config.debug,
        "gwt started"
    );

    match cli.command {
        Some(cmd) => commands::handle_command(cmd, &repo_root, &settings),
        None => {
            let mut entry: Option<TuiEntryContext> = None;
            loop {
                let selection = tui::run_with_context(entry.take())?;
                match selection {
                    Some(launch_plan) => {
                        // FR-088: Record agent usage to history (single mode)
                        // Run in background thread to avoid blocking agent startup
                        let history_repo_root = repo_root.clone();
                        let history_branch_name = launch_plan.config.branch_name.clone();
                        // T603: Use custom agent ID/label when available (SPEC-71f2742d US6)
                        let (history_agent_id, history_agent_label) = if let Some(ref custom) =
                            launch_plan.config.custom_agent
                        {
                            (
                                custom.id.clone(),
                                format!("{}@{}", custom.display_name, launch_plan.selected_version),
                            )
                        } else {
                            (
                                launch_plan.config.agent.id().to_string(),
                                format!(
                                    "{}@{}",
                                    launch_plan.config.agent.label(),
                                    launch_plan.selected_version
                                ),
                            )
                        };
                        thread::spawn(move || {
                            let mut agent_history = AgentHistoryStore::load().unwrap_or_default();
                            if let Err(e) = agent_history.record(
                                &history_repo_root,
                                &history_branch_name,
                                &history_agent_id,
                                &history_agent_label,
                            ) {
                                warn!(category = "main", "Failed to record agent history: {}", e);
                            }
                            if let Err(e) = agent_history.save() {
                                warn!(category = "main", "Failed to save agent history: {}", e);
                            }
                        });
                        // SPEC-a70a1ece: Capture repo_root before launch_plan is consumed
                        let entry_repo_root = launch_plan.repo_root.clone();
                        match execute_launch_plan(launch_plan) {
                            Ok(AgentExitKind::Success) => {
                                entry = Some(
                                    TuiEntryContext::success(
                                        "Session completed successfully.".to_string(),
                                    )
                                    .with_repo_root(entry_repo_root),
                                );
                            }
                            Ok(AgentExitKind::Interrupted) => {
                                entry = Some(
                                    TuiEntryContext::warning("Session interrupted.".to_string())
                                        .with_repo_root(entry_repo_root),
                                );
                            }
                            Err(err) => {
                                entry = Some(
                                    TuiEntryContext::error(err.to_string())
                                        .with_repo_root(entry_repo_root),
                                );
                            }
                        }
                    }
                    None => break,
                }
            }
            Ok(())
        }
    }
}

fn cleanup_startup_logs(repo_root: &Path, settings: &Settings) -> Result<usize, GwtError> {
    let log_dir = settings.log_dir(repo_root);
    gwt_core::logging::cleanup_old_logs(&log_dir, settings.log_retention_days)
}

/// Map hook event name to AgentStatus (SPEC-861d8cdf T-101)
///
/// Event mappings:
/// - UserPromptSubmit, PreToolUse, PostToolUse, SessionStart -> Running
/// - Stop, SubagentStop, SessionEnd -> Stopped
/// - Notification (with permission_prompt type) -> WaitingInput
/// - Unknown events -> Running (activity indicator)
fn hook_event_to_status(event: &str, payload: &serde_json::Value) -> AgentStatus {
    match event.to_lowercase().as_str() {
        "userpromptsubmit" | "pretooluse" | "posttooluse" => AgentStatus::Running,
        "stop" | "subagentstop" => AgentStatus::Stopped,
        "notification" => {
            // Check if this is a permission prompt notification
            let notification_type = payload
                .get("notification")
                .and_then(|n| n.get("type"))
                .and_then(|t| t.as_str())
                .unwrap_or("");

            if notification_type == "permission_prompt" {
                AgentStatus::WaitingInput
            } else {
                // Other notifications indicate activity
                AgentStatus::Running
            }
        }
        "sessionstart" => AgentStatus::Running,
        "sessionend" => AgentStatus::Stopped,
        _ => {
            // Unknown events are treated as activity
            AgentStatus::Running
        }
    }
}

/// Detect branch name from worktree path
fn detect_branch_name(path: &Path) -> String {
    // Try to get branch from git
    let output = Command::new("git")
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .current_dir(path)
        .output();

    match output {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => {
            // Fallback: extract from path
            path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        }
    }
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
fn should_warn_skip_install(worktree_path: &Path) -> Option<&'static str> {
    if !worktree_path.join("package.json").exists() {
        return None;
    }
    if worktree_path.join("node_modules").exists() {
        return None;
    }
    detect_package_manager(worktree_path)
}

fn skip_install_warning_message(pm: &str) -> String {
    format!(
        "Auto install disabled. Skipping dependency install. Run \"{} install\" if needed or set GWT_AGENT_AUTO_INSTALL_DEPS=true.",
        pm
    )
}

const FAST_EXIT_THRESHOLD_SECS: u64 = 2;
const FAST_EXIT_THRESHOLD_MS: u128 = (FAST_EXIT_THRESHOLD_SECS as u128) * 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AgentExitKind {
    Success,
    Interrupted,
}

#[derive(Debug, Clone)]
struct SessionUpdateContext {
    worktree_path: PathBuf,
    branch_name: String,
    agent_id: String,
    agent_label: String,
    version: String,
    model: Option<String>,
    mode: String,
    reasoning_level: Option<String>,
    skip_permissions: bool,
    collaboration_modes: bool,
}

impl SessionUpdateContext {
    fn to_entry(&self) -> ToolSessionEntry {
        ToolSessionEntry {
            branch: self.branch_name.clone(),
            worktree_path: Some(self.worktree_path.to_string_lossy().to_string()),
            tool_id: self.agent_id.clone(),
            tool_label: self.agent_label.clone(),
            session_id: None,
            mode: Some(self.mode.clone()),
            model: self.model.clone(),
            reasoning_level: self.reasoning_level.clone(),
            skip_permissions: Some(self.skip_permissions),
            tool_version: Some(self.version.clone()),
            collaboration_modes: Some(self.collaboration_modes),
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            timestamp: Utc::now().timestamp_millis(),
        }
    }
}

struct SessionUpdater {
    stop_tx: mpsc::Sender<()>,
    handle: thread::JoinHandle<()>,
}

impl SessionUpdater {
    fn stop(self) {
        let _ = self.stop_tx.send(());
        let _ = self.handle.join();
    }
}

fn spawn_session_updater(context: SessionUpdateContext, interval: Duration) -> SessionUpdater {
    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = thread::spawn(move || loop {
        match stop_rx.recv_timeout(interval) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => {
                let entry = context.to_entry();
                let _ = save_session_entry(&context.worktree_path, entry);
            }
        }
    });

    SessionUpdater { stop_tx, handle }
}

fn build_launching_message(config: &AgentLaunchConfig) -> String {
    format!("Launching {}...", config.agent.label())
}

fn is_fast_exit(duration_ms: u128) -> bool {
    duration_ms < FAST_EXIT_THRESHOLD_MS
}

fn format_command_line(executable: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(executable.to_string());
    parts.extend(args.iter().cloned());
    parts.join(" ")
}

fn emit_fast_exit_notice(duration_ms: u128, command_display: &str) {
    eprintln!("Agent exited immediately ({} ms).", duration_ms);
    eprintln!("This usually means the agent could not start.");
    eprintln!("Check API keys, PATH, and tool version, then try running:");
    eprintln!("  {}", command_display);
    if io::stdin().is_terminal() {
        eprintln!();
        eprintln!("Press Enter to return to gwt.");
        let _ = io::stderr().flush();
        let mut input = String::new();
        let _ = io::stdin().read_line(&mut input);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LaunchProgress {
    #[allow(dead_code)]
    ResolvingWorktree,
    BuildingCommand,
    CheckingDependencies,
    InstallingDependencies {
        manager: String,
    },
}

impl LaunchProgress {
    pub(crate) fn message(&self) -> String {
        match self {
            LaunchProgress::ResolvingWorktree => "Preparing worktree...".to_string(),
            LaunchProgress::BuildingCommand => "Preparing launch command...".to_string(),
            LaunchProgress::CheckingDependencies => "Checking dependencies...".to_string(),
            LaunchProgress::InstallingDependencies { manager } => {
                format!("Installing dependencies with {}...", manager)
            }
        }
    }
}

/// Progress step kind for worktree preparation modal (FR-048)
/// The 6 stages of worktree preparation process
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressStepKind {
    /// 1. Fetching remote...
    FetchRemote,
    /// 2. Validating branch...
    ValidateBranch,
    /// 3. Generating path...
    GeneratePath,
    /// 4. Checking conflicts...
    CheckConflicts,
    /// 5. Creating worktree...
    CreateWorktree,
    /// 6. Checking dependencies...
    CheckDependencies,
}

impl ProgressStepKind {
    /// Returns the display message for this step kind
    pub(crate) fn message(&self) -> &'static str {
        match self {
            ProgressStepKind::FetchRemote => "Fetching remote...",
            ProgressStepKind::ValidateBranch => "Validating branch...",
            ProgressStepKind::GeneratePath => "Generating path...",
            ProgressStepKind::CheckConflicts => "Checking conflicts...",
            ProgressStepKind::CreateWorktree => "Creating worktree...",
            ProgressStepKind::CheckDependencies => "Checking dependencies...",
        }
    }

    /// Returns all step kinds in order
    pub(crate) fn all() -> [ProgressStepKind; 6] {
        [
            ProgressStepKind::FetchRemote,
            ProgressStepKind::ValidateBranch,
            ProgressStepKind::GeneratePath,
            ProgressStepKind::CheckConflicts,
            ProgressStepKind::CreateWorktree,
            ProgressStepKind::CheckDependencies,
        ]
    }
}

/// Step status for progress modal (FR-047)
/// [x] Completed, [>] Running, [ ] Pending, [!] Failed, [skip] Skipped
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StepStatus {
    /// [ ] - Waiting to start
    #[default]
    Pending,
    /// [>] - Currently executing
    Running,
    /// [x] - Successfully completed
    Completed,
    /// [!] - Failed with error
    Failed,
    /// [skip] - Skipped (e.g., existing worktree reuse)
    Skipped,
}

/// Individual progress step with timing and error info (FR-049)
#[derive(Debug, Clone)]
pub(crate) struct ProgressStep {
    pub kind: ProgressStepKind,
    pub status: StepStatus,
    pub started_at: Option<Instant>,
    pub error_message: Option<String>,
}

impl ProgressStep {
    /// Create a new pending step
    pub(crate) fn new(kind: ProgressStepKind) -> Self {
        Self {
            kind,
            status: StepStatus::Pending,
            started_at: None,
            error_message: None,
        }
    }

    /// Start this step (Pending -> Running)
    pub(crate) fn start(&mut self) {
        self.status = StepStatus::Running;
        self.started_at = Some(Instant::now());
    }

    /// Complete this step (Running -> Completed)
    pub(crate) fn complete(&mut self) {
        self.status = StepStatus::Completed;
    }

    /// Mark this step as failed with an error message
    pub(crate) fn fail(&mut self, message: String) {
        self.status = StepStatus::Failed;
        self.error_message = Some(message);
    }

    /// Skip this step (for existing worktree reuse)
    pub(crate) fn skip(&mut self) {
        self.status = StepStatus::Skipped;
    }

    /// Get the marker string for display (FR-047)
    pub(crate) fn marker(&self) -> &'static str {
        match self.status {
            StepStatus::Pending => "[ ]",
            StepStatus::Running => "[>]",
            StepStatus::Completed => "[x]",
            StepStatus::Failed => "[!]",
            StepStatus::Skipped => "[skip]",
        }
    }

    /// Get elapsed seconds since start (if started)
    pub(crate) fn elapsed_secs(&self) -> Option<f64> {
        self.started_at.map(|t| t.elapsed().as_secs_f64())
    }

    /// Check if elapsed time should be shown (>= 3 seconds) (FR-049)
    pub(crate) fn should_show_elapsed(&self) -> bool {
        self.elapsed_secs().is_some_and(|secs| secs >= 3.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum InstallPlan {
    None,
    Skip { message: String },
    Install { manager: String },
}

#[derive(Debug, Clone)]
pub(crate) struct LaunchPlan {
    pub config: AgentLaunchConfig,
    pub executable: String,
    pub command_args: Vec<String>,
    pub log_lines: Vec<String>,
    pub session_warning: Option<String>,
    pub selected_version: String,
    pub install_plan: InstallPlan,
    pub env: Vec<(String, String)>,
    /// Repository root for single mode re-entry (SPEC-a70a1ece)
    pub repo_root: PathBuf,
}

fn should_set_claude_sandbox_env(target_os: &str) -> bool {
    target_os != "windows"
}

pub(crate) fn build_launch_env(config: &AgentLaunchConfig) -> Vec<(String, String)> {
    let mut env_vars = config.env.clone();
    if config.skip_permissions && config.agent == CodingAgent::ClaudeCode {
        let has_sandbox = env_vars.iter().any(|(key, _)| key == "IS_SANDBOX");
        if !has_sandbox && should_set_claude_sandbox_env(std::env::consts::OS) {
            env_vars.push(("IS_SANDBOX".to_string(), "1".to_string()));
        }
    }
    env_vars
}

fn build_install_plan(worktree_path: &Path, auto_install: bool) -> InstallPlan {
    if !worktree_path.join("package.json").exists() {
        return InstallPlan::None;
    }

    if !auto_install {
        if let Some(pm) = should_warn_skip_install(worktree_path) {
            return InstallPlan::Skip {
                message: skip_install_warning_message(pm),
            };
        }
        return InstallPlan::None;
    }

    if worktree_path.join("node_modules").exists() {
        return InstallPlan::None;
    }

    let pm = match detect_package_manager(worktree_path) {
        Some(pm) => pm,
        None => return InstallPlan::None,
    };

    InstallPlan::Install {
        manager: pm.to_string(),
    }
}

fn run_install_plan(worktree_path: &Path, plan: &InstallPlan) -> Result<(), GwtError> {
    match plan {
        InstallPlan::None => Ok(()),
        InstallPlan::Skip { message } => {
            println!("{}", message);
            println!();
            Ok(())
        }
        InstallPlan::Install { manager } => {
            println!("Installing dependencies with {}...", manager);
            println!();
            let status = Command::new(manager)
                .arg("install")
                .current_dir(worktree_path)
                .status()
                .map_err(|e| GwtError::AgentLaunchFailed {
                    name: manager.to_string(),
                    reason: format!("Failed to run '{}': {}", manager, e),
                })?;
            if !status.success() {
                eprintln!();
                eprintln!(
                    "Warning: {} install exited with status: {}",
                    manager, status
                );
                // Don't fail - let the user decide whether to continue
            } else {
                println!();
                println!("Dependencies installed successfully.");
            }

            println!();
            Ok(())
        }
    }
}

/// Prepare a launch plan for a coding agent
///
/// Version selection behavior (FR-066, FR-067, FR-068):
/// - "installed": Use local command if available, fallback to bunx @package@latest
/// - "latest": Use bunx @package@latest
/// - specific version: Use bunx @package@X.Y.Z
pub(crate) fn prepare_launch_plan(
    config: AgentLaunchConfig,
    mut progress: impl FnMut(LaunchProgress),
) -> Result<LaunchPlan, GwtError> {
    progress(LaunchProgress::BuildingCommand);

    let (config, session_warning) = normalize_session_id_for_launch(config);

    // SPEC-71f2742d: Handle custom agents (T207)
    if config.custom_agent.is_some() {
        return prepare_custom_agent_launch_plan(config, session_warning, progress);
    }

    let env = build_launch_env(&config);

    let cmd_name = config.agent.command_name();
    let npm_package = config.agent.npm_package();

    // Determine execution method based on version selection
    let (executable, base_args, using_local) = if config.version == "installed" {
        // FR-066: Try local command first
        match which::which(cmd_name) {
            Ok(path) => (path.to_string_lossy().to_string(), vec![], true),
            Err(_) => {
                // FR-019: Fallback to bunx @package@latest if local not found
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
    let version_label = selected_version.clone();

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

    let log_lines = build_launch_log_lines(
        &config,
        &agent_args,
        &version_label,
        &execution_method,
        &env,
    );

    progress(LaunchProgress::CheckingDependencies);
    let install_plan = build_install_plan(&config.worktree_path, config.auto_install_deps);
    if let InstallPlan::Install { manager } = &install_plan {
        progress(LaunchProgress::InstallingDependencies {
            manager: manager.clone(),
        });
    }

    // SPEC-a70a1ece: Capture repo_root for single mode re-entry
    let repo_root = config.repo_root.clone();

    Ok(LaunchPlan {
        config,
        executable,
        command_args,
        log_lines,
        session_warning,
        selected_version,
        install_plan,
        env,
        repo_root,
    })
}

/// Prepare a launch plan for a custom coding agent (SPEC-71f2742d T207)
fn prepare_custom_agent_launch_plan(
    config: AgentLaunchConfig,
    session_warning: Option<String>,
    mut progress: impl FnMut(LaunchProgress),
) -> Result<LaunchPlan, GwtError> {
    use gwt_core::config::AgentType;

    let custom = config
        .custom_agent
        .as_ref()
        .ok_or_else(|| GwtError::AgentConfigInvalid {
            name: config.agent.label().to_string(),
        })?;

    // Build environment including custom env vars (T209)
    let mut env = build_launch_env(&config);
    for (key, value) in &custom.env {
        env.push((key.clone(), value.clone()));
    }

    // Determine executable and base args based on agent type (T207)
    let (executable, base_args) = match custom.agent_type {
        AgentType::Command => {
            // PATH search for command
            match which::which(&custom.command) {
                Ok(path) => (path.to_string_lossy().to_string(), vec![]),
                Err(_) => {
                    return Err(GwtError::AgentNotFound {
                        name: custom.command.clone(),
                    });
                }
            }
        }
        AgentType::Path => {
            // Use absolute path directly
            let path = std::path::Path::new(&custom.command);
            if !path.exists() {
                return Err(GwtError::AgentNotFound {
                    name: custom.command.clone(),
                });
            }
            (custom.command.clone(), vec![])
        }
        AgentType::Bunx => {
            // Use bunx to run the command
            get_bunx_command(&custom.command, "latest")
        }
    };

    // Build agent-specific arguments
    let agent_args = build_agent_args(&config);
    let mut command_args = base_args;
    command_args.extend(agent_args.clone());

    let version_label = "custom".to_string();
    let execution_method = ExecutionMethod::Installed {
        command: custom.command.clone(),
    };

    let log_lines = build_launch_log_lines(
        &config,
        &agent_args,
        &version_label,
        &execution_method,
        &env,
    );

    progress(LaunchProgress::CheckingDependencies);
    let install_plan = build_install_plan(&config.worktree_path, config.auto_install_deps);
    if let InstallPlan::Install { manager } = &install_plan {
        progress(LaunchProgress::InstallingDependencies {
            manager: manager.clone(),
        });
    }

    // SPEC-a70a1ece: Capture repo_root for single mode re-entry
    let repo_root = config.repo_root.clone();

    Ok(LaunchPlan {
        config,
        executable,
        command_args,
        log_lines,
        session_warning,
        selected_version: version_label,
        install_plan,
        env,
        repo_root,
    })
}

fn execute_launch_plan(plan: LaunchPlan) -> Result<AgentExitKind, GwtError> {
    let LaunchPlan {
        config,
        executable,
        command_args,
        log_lines,
        session_warning,
        selected_version,
        install_plan,
        env,
        ..
    } = plan;
    println!("{}", build_launching_message(&config));
    println!();
    if let Some(warning) = session_warning.as_ref() {
        eprintln!("{}", warning);
        eprintln!();
    }
    if config.version == "installed" && selected_version == "latest" {
        eprintln!(
            "Note: Local '{}' not found, using bunx fallback",
            config.agent.command_name()
        );
    }

    for line in &log_lines {
        println!("{}", line);
    }
    println!();

    if config.auto_install_deps && matches!(install_plan, InstallPlan::Install { .. }) {
        println!("Preparing to install dependencies...");
        println!();
    }

    // FR-040a/FR-040b: Install dependencies only when enabled
    run_install_plan(&config.worktree_path, &install_plan)?;
    let started_at = Instant::now();

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
        collaboration_modes: Some(config.collaboration_modes),
        docker_service: None,
        docker_force_host: None,
        docker_recreate: None,
        docker_build: None,
        docker_keep: None,
        timestamp: Utc::now().timestamp_millis(),
    };
    if let Err(e) = save_session_entry(&config.worktree_path, session_entry) {
        eprintln!("Warning: Failed to save session: {}", e);
    }

    // Spawn the agent process (FR-043: allows periodic timestamp updates)
    let (exec_name, exec_args) = apply_pty_wrapper(&executable, &command_args);
    let command_display = format_command_line(&exec_name, &exec_args);
    let mut command = Command::new(&exec_name);
    command
        .args(&exec_args)
        .current_dir(&config.worktree_path)
        .stdin(Stdio::inherit()) // Keep stdin for interactive input
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    apply_tty_stdio(&mut command);
    for key in &config.env_remove {
        command.env_remove(key);
    }
    for (key, value) in &env {
        command.env(key, value);
    }

    let mut child = command.spawn().map_err(|e| GwtError::AgentLaunchFailed {
        name: config.agent.command_name().to_string(),
        reason: format!("Failed to execute '{}': {}", executable, e),
    })?;

    debug!(
        category = "agent",
        agent_id = config.agent.id(),
        worktree_path = %config.worktree_path.display(),
        "Agent process started"
    );

    // FR-043: Start background thread to update timestamp every 30 seconds
    let update_context = SessionUpdateContext {
        worktree_path: config.worktree_path.clone(),
        branch_name: config.branch_name.clone(),
        agent_id: config.agent.id().to_string(),
        agent_label: config.agent.label().to_string(),
        version: selected_version.clone(),
        model: config.model.clone(),
        mode: config.execution_mode.label().to_string(),
        reasoning_level: config.reasoning_level.map(|r| r.label().to_string()),
        skip_permissions: config.skip_permissions,
        collaboration_modes: config.collaboration_modes,
    };
    let updater = spawn_session_updater(update_context, Duration::from_secs(30));

    // Wait for the agent process to finish
    let status = child.wait().map_err(|e| GwtError::AgentLaunchFailed {
        name: config.agent.command_name().to_string(),
        reason: format!("Failed to wait for '{}': {}", executable, e),
    })?;

    // Signal the updater thread to stop and wait for it
    updater.stop();

    debug!(
        category = "agent",
        agent_id = config.agent.id(),
        "Agent process finished"
    );

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
            collaboration_modes: Some(config.collaboration_modes),
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            timestamp: Utc::now().timestamp_millis(),
        };
        if let Err(e) = save_session_entry(&config.worktree_path, entry) {
            eprintln!("Warning: Failed to save session: {}", e);
        }
    }

    let duration_ms = started_at.elapsed().as_millis();
    let fast_exit = is_fast_exit(duration_ms);
    let exit_info = classify_exit_status(status);
    match exit_info {
        ExitClassification::Success => {
            if fast_exit {
                warn!(
                    agent_id = config.agent.id(),
                    version = selected_version.as_str(),
                    duration_ms = duration_ms as u64,
                    "Agent exited immediately"
                );
                emit_fast_exit_notice(duration_ms, &command_display);
                return Err(GwtError::AgentLaunchFailed {
                    name: config.agent.command_name().to_string(),
                    reason: format!("Exited immediately after {} ms.", duration_ms),
                });
            }
            info!(
                agent_id = config.agent.id(),
                version = selected_version.as_str(),
                duration_ms = duration_ms as u64,
                "Agent exited successfully"
            );
            Ok(AgentExitKind::Success)
        }
        ExitClassification::Interrupted => {
            warn!(
                agent_id = config.agent.id(),
                version = selected_version.as_str(),
                duration_ms = duration_ms as u64,
                "Agent session interrupted"
            );
            Ok(AgentExitKind::Interrupted)
        }
        ExitClassification::Failure { code, signal } => {
            error!(
                agent_id = config.agent.id(),
                version = selected_version.as_str(),
                exit_code = code,
                signal = signal,
                duration_ms = duration_ms as u64,
                "Agent exited with failure"
            );
            if fast_exit {
                emit_fast_exit_notice(duration_ms, &command_display);
            }
            let reason = if let Some(signal) = signal {
                format!("Terminated by signal {}", signal)
            } else if let Some(code) = code {
                format_exit_code(code)
            } else {
                "Exited with unknown status".to_string()
            };
            Err(GwtError::AgentLaunchFailed {
                name: config.agent.command_name().to_string(),
                reason,
            })
        }
    }
}

/// Get bunx command and base args for npm package execution
fn get_bunx_command(npm_package: &str, version: &str) -> (String, Vec<String>) {
    // Try bunx first, but avoid project-local node_modules/.bin shims.
    let bunx_path = which::which("bunx").ok();
    let npx_path = which::which("npx").ok();
    let runner_path =
        select_runner_executable(bunx_path, npx_path).unwrap_or_else(|| PathBuf::from("bunx"));
    let runner_path_str = runner_path.to_string_lossy().to_string();

    let package_spec = if version == "latest" {
        format!("{}@latest", npm_package)
    } else {
        format!("{}@{}", npm_package, version)
    };

    (
        runner_path_str.clone(),
        build_runner_args(package_spec, &runner_path_str),
    )
}

fn select_runner_executable(
    bunx_path: Option<PathBuf>,
    npx_path: Option<PathBuf>,
) -> Option<PathBuf> {
    if let Some(path) = bunx_path.as_ref() {
        if !is_node_modules_bin(path) {
            return bunx_path;
        }
    }
    if npx_path.is_some() {
        return npx_path;
    }
    bunx_path
}

fn is_node_modules_bin(path: &Path) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.contains("/node_modules/.bin/")
}

fn build_runner_args(package_spec: String, runner_executable: &str) -> Vec<String> {
    let mut args = Vec::new();
    if runner_requires_yes(runner_executable) {
        args.push("--yes".to_string());
    }
    args.push(package_spec);
    args
}

fn runner_requires_yes(executable: &str) -> bool {
    std::path::Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "npx" | "npx.cmd" | "npx.exe"
            )
        })
        .unwrap_or(false)
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
        ExecutionMode::Convert => "Convert session",
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

fn format_env_log_lines(env_vars: &[(String, String)]) -> Vec<String> {
    if env_vars.is_empty() {
        return vec!["Env: (none)".to_string()];
    }
    let mut vars: Vec<(String, String)> = env_vars
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    vars.sort_by(|a, b| a.0.cmp(&b.0));
    let mut lines = Vec::with_capacity(vars.len() + 1);
    lines.push("Env:".to_string());
    for (key, value) in vars {
        lines.push(format!("  {}={}", key, value));
    }
    lines
}

fn build_launch_log_lines(
    config: &AgentLaunchConfig,
    agent_args: &[String],
    version_label: &str,
    execution_method: &ExecutionMethod,
    env_vars: &[(String, String)],
) -> Vec<String> {
    let mut lines = Vec::new();
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
    lines.extend(format_env_log_lines(env_vars));
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

fn session_exists_for_tool_at(home: &Path, tool_id: &str, session_id: &str) -> Option<bool> {
    let agent = SessionAgentType::from_tool_id(tool_id)?;
    let exists = match agent {
        SessionAgentType::ClaudeCode => {
            ClaudeSessionParser::new(home.to_path_buf()).session_exists(session_id)
        }
        SessionAgentType::CodexCli => {
            CodexSessionParser::new(home.to_path_buf()).session_exists(session_id)
        }
        SessionAgentType::GeminiCli => {
            GeminiSessionParser::new(home.to_path_buf()).session_exists(session_id)
        }
        SessionAgentType::OpenCode => {
            OpenCodeSessionParser::new(home.to_path_buf()).session_exists(session_id)
        }
    };
    Some(exists)
}

fn normalize_session_id_for_launch_with_home(
    mut config: AgentLaunchConfig,
    home: Option<PathBuf>,
) -> (AgentLaunchConfig, Option<String>) {
    if config.custom_agent.is_some() {
        return (config, None);
    }

    if !matches!(
        config.execution_mode,
        ExecutionMode::Continue | ExecutionMode::Resume
    ) {
        return (config, None);
    }

    let Some(session_id) = config.session_id.clone().filter(|id| !id.trim().is_empty()) else {
        return (config, None);
    };

    let Some(home) = home else {
        return (config, None);
    };

    let Some(exists) = session_exists_for_tool_at(&home, config.agent.id(), &session_id) else {
        return (config, None);
    };

    if exists {
        return (config, None);
    }

    config.session_id = None;
    let warning = format!(
        "Warning: Saved session '{}' not found; falling back to default resume behavior.",
        session_id
    );
    (config, Some(warning))
}

fn normalize_session_id_for_launch(
    config: AgentLaunchConfig,
) -> (AgentLaunchConfig, Option<String>) {
    normalize_session_id_for_launch_with_home(config, home_dir())
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

// ============================================================================
// History.jsonl schema definitions and parsers
// ============================================================================

/// Claude Code history.jsonl entry schema
#[derive(Debug, Clone, serde::Deserialize)]
struct ClaudeHistoryEntry {
    #[allow(dead_code)]
    display: String,
    #[serde(rename = "pastedContents")]
    #[allow(dead_code)]
    pasted_contents: serde_json::Value,
    timestamp: u64, // milliseconds
    project: String,
    #[serde(rename = "sessionId")]
    session_id: String,
}

/// Codex history.jsonl entry schema
#[derive(Debug, Clone, serde::Deserialize)]
struct CodexHistoryEntry {
    session_id: String,
    ts: u64, // seconds
    #[allow(dead_code)]
    text: String,
}

/// Parse Claude Code history.jsonl file
fn parse_claude_history(home: &Path) -> Vec<ClaudeHistoryEntry> {
    let path = home.join(".claude").join("history.jsonl");
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let reader = std::io::BufReader::new(file);
    reader
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// Parse Codex history.jsonl file
fn parse_codex_history(home: &Path) -> Vec<CodexHistoryEntry> {
    let path = home.join(".codex").join("history.jsonl");
    let file = match fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return vec![],
    };
    let reader = std::io::BufReader::new(file);
    reader
        .lines()
        .map_while(Result::ok)
        .filter_map(|line| serde_json::from_str(&line).ok())
        .collect()
}

/// Get latest session ID for Claude Code from history.jsonl
fn get_latest_claude_session_id(home: &Path, worktree_path: &Path) -> Option<String> {
    let history = parse_claude_history(home);
    let worktree_str = worktree_path.to_string_lossy();

    history
        .iter()
        .filter(|e| e.project == worktree_str || worktree_str.ends_with(&e.project))
        .max_by_key(|e| e.timestamp)
        .map(|e| e.session_id.clone())
}

/// Get latest session ID for Codex from history.jsonl
/// Note: Codex history.jsonl does not have project field, so we return the latest session
fn get_latest_codex_session_id(home: &Path) -> Option<String> {
    let history = parse_codex_history(home);
    history
        .iter()
        .max_by_key(|e| e.ts)
        .map(|e| e.session_id.clone())
}

// ============================================================================
// Generic session parsers (legacy)
// ============================================================================

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
    // Primary: Scan sessions/ directory with cwd matching
    let root = home.join(".codex").join("sessions");
    if root.exists() {
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

        if let Some((_, session_id)) = latest_match.or(latest_any) {
            return Some(session_id);
        }
    }

    // Fallback: Use history.jsonl (returns latest session regardless of project)
    get_latest_codex_session_id(home)
}

fn detect_claude_session_id_at(home: &Path, worktree_path: &Path) -> Option<String> {
    // Primary: Use history.jsonl which has accurate project -> sessionId mapping
    if let Some(session_id) = get_latest_claude_session_id(home, worktree_path) {
        return Some(session_id);
    }

    // Fallback: Scan projects/ directory (legacy behavior)
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

pub(crate) fn detect_session_id_for_tool(tool_id: &str, worktree_path: &Path) -> Option<String> {
    let home = home_dir()?;
    let lower = tool_id.to_lowercase();
    if lower.contains("codex") {
        return detect_codex_session_id_at(&home, worktree_path);
    }
    if lower.contains("claude") {
        return detect_claude_session_id_at(&home, worktree_path);
    }
    if lower.contains("gemini") {
        return detect_gemini_session_id_at(&home);
    }
    if lower.contains("opencode") || lower.contains("open-code") {
        return detect_opencode_session_id_at(&home);
    }
    None
}

fn detect_agent_session_id(config: &AgentLaunchConfig) -> Option<String> {
    detect_session_id_for_tool(config.agent.id(), &config.worktree_path)
}

fn apply_tty_stdio(command: &mut Command) {
    #[cfg(unix)]
    {
        let stdin = OpenOptions::new().read(true).open("/dev/tty");
        let stdout = OpenOptions::new().write(true).open("/dev/tty");
        let stderr = OpenOptions::new().write(true).open("/dev/tty");
        if let Ok(file) = stdin {
            command.stdin(Stdio::from(file));
        }
        if let Ok(file) = stdout {
            command.stdout(Stdio::from(file));
        }
        if let Ok(file) = stderr {
            command.stderr(Stdio::from(file));
        }
    }
}

fn apply_pty_wrapper(executable: &str, args: &[String]) -> (String, Vec<String>) {
    #[cfg(target_os = "macos")]
    {
        let mut wrapped_args = Vec::new();
        wrapped_args.push("-q".to_string());
        wrapped_args.push("/dev/null".to_string());
        wrapped_args.push(executable.to_string());
        wrapped_args.extend(args.iter().cloned());
        ("script".to_string(), wrapped_args)
    }

    #[cfg(not(target_os = "macos"))]
    {
        (executable.to_string(), args.to_vec())
    }
}

/// Build agent-specific command line arguments
fn build_agent_args(config: &AgentLaunchConfig) -> Vec<String> {
    // SPEC-71f2742d: Handle custom agents
    if let Some(ref custom) = config.custom_agent {
        return build_custom_agent_args(custom, config);
    }

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
                ExecutionMode::Continue | ExecutionMode::Resume | ExecutionMode::Convert => {
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
                ExecutionMode::Continue | ExecutionMode::Resume | ExecutionMode::Convert => {
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
                config.collaboration_modes,
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
                ExecutionMode::Continue | ExecutionMode::Resume | ExecutionMode::Convert => {
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
                ExecutionMode::Resume | ExecutionMode::Convert => {
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

/// Build command line arguments for custom agents (SPEC-71f2742d T206)
fn build_custom_agent_args(
    custom: &gwt_core::config::CustomCodingAgent,
    config: &AgentLaunchConfig,
) -> Vec<String> {
    let mut args = Vec::new();

    // Add default args
    args.extend(custom.default_args.clone());

    // Add mode-specific args (T208)
    if let Some(ref mode_args) = custom.mode_args {
        match config.execution_mode {
            ExecutionMode::Normal => {
                args.extend(mode_args.normal.clone());
            }
            ExecutionMode::Continue => {
                args.extend(mode_args.continue_mode.clone());
            }
            ExecutionMode::Resume | ExecutionMode::Convert => {
                args.extend(mode_args.resume.clone());
            }
        }
    }

    // Add permission skip args if skip_permissions is true (T210)
    if config.skip_permissions && !custom.permission_skip_args.is_empty() {
        args.extend(custom.permission_skip_args.clone());
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
            repo_root: PathBuf::from("/tmp/repo"),
            worktree_path: PathBuf::from("/tmp/worktree"),
            branch_name: "feature/test".to_string(),
            agent,
            custom_agent: None,
            model: Some("sonnet".to_string()),
            reasoning_level: None,
            version: "latest".to_string(),
            execution_mode: ExecutionMode::Continue,
            session_id: None,
            skip_permissions: true,
            env: Vec::new(),
            env_remove: Vec::new(),
            auto_install_deps: false,
            collaboration_modes: false,
        }
    }

    fn sample_update_context(path: PathBuf) -> SessionUpdateContext {
        SessionUpdateContext {
            worktree_path: path,
            branch_name: "feature/test".to_string(),
            agent_id: "codex-cli".to_string(),
            agent_label: "Codex".to_string(),
            version: "latest".to_string(),
            model: None,
            mode: ExecutionMode::Continue.label().to_string(),
            reasoning_level: None,
            skip_permissions: false,
            collaboration_modes: true,
        }
    }

    #[test]
    fn test_cleanup_startup_logs_removes_old_logs() {
        let temp = TempDir::new().unwrap();
        let log_dir = temp.path().join("logs");
        fs::create_dir_all(&log_dir).unwrap();
        fs::write(log_dir.join("gwt.jsonl.2024-01-01"), "old").unwrap();

        let settings = Settings {
            log_dir: Some(log_dir),
            log_retention_days: 0,
            ..Default::default()
        };

        let removed = cleanup_startup_logs(temp.path(), &settings).unwrap();
        assert!(removed >= 1);
    }

    #[test]
    fn test_prepare_custom_agent_launch_plan_requires_custom_agent() {
        let config = sample_config(CodingAgent::CodexCli);
        let result = prepare_custom_agent_launch_plan(config, None, |_| {});
        assert!(matches!(result, Err(GwtError::AgentConfigInvalid { .. })));
    }

    #[test]
    fn test_should_set_claude_sandbox_env_windows() {
        assert!(!should_set_claude_sandbox_env("windows"));
    }

    #[test]
    fn test_should_set_claude_sandbox_env_non_windows() {
        assert!(should_set_claude_sandbox_env("linux"));
        assert!(should_set_claude_sandbox_env("macos"));
    }

    #[test]
    fn test_is_node_modules_bin_detects_paths() {
        let unix_path = Path::new("/repo/node_modules/.bin/bunx");
        let windows_path = Path::new("C:\\repo\\node_modules\\.bin\\bunx");
        assert!(is_node_modules_bin(unix_path));
        assert!(is_node_modules_bin(windows_path));
    }

    #[test]
    fn test_select_runner_executable_prefers_global_bunx() {
        let bunx = PathBuf::from("/usr/local/bin/bunx");
        let npx = PathBuf::from("/usr/bin/npx");
        let selected = select_runner_executable(Some(bunx.clone()), Some(npx));
        assert_eq!(selected, Some(bunx));
    }

    #[test]
    fn test_select_runner_executable_skips_local_bunx_prefers_npx() {
        let bunx = PathBuf::from("/repo/node_modules/.bin/bunx");
        let npx = PathBuf::from("/usr/bin/npx");
        let selected = select_runner_executable(Some(bunx), Some(npx.clone()));
        assert_eq!(selected, Some(npx));
    }

    #[test]
    fn test_select_runner_executable_falls_back_to_local_bunx() {
        let bunx = PathBuf::from("/repo/node_modules/.bin/bunx");
        let selected = select_runner_executable(Some(bunx.clone()), None);
        assert_eq!(selected, Some(bunx));
    }

    #[test]
    fn test_runner_requires_yes_for_npx_variants() {
        assert!(runner_requires_yes("npx"));
        assert!(runner_requires_yes("/usr/local/bin/npx"));
        assert!(runner_requires_yes("npx.cmd"));
        assert!(runner_requires_yes("npx.exe"));
        assert!(!runner_requires_yes("bunx"));
    }

    #[test]
    fn test_build_runner_args_adds_yes_for_npx() {
        let args = build_runner_args("@openai/codex@latest".to_string(), "npx");
        assert_eq!(
            args,
            vec!["--yes".to_string(), "@openai/codex@latest".to_string()]
        );
    }

    #[test]
    fn test_build_runner_args_skips_yes_for_bunx() {
        let args = build_runner_args("@openai/codex@latest".to_string(), "bunx");
        assert_eq!(args, vec!["@openai/codex@latest".to_string()]);
    }

    #[test]
    fn test_build_launch_env_claude_skip_permissions_gates_is_sandbox() {
        let config = sample_config(CodingAgent::ClaudeCode);
        let env = build_launch_env(&config);
        let has_sandbox = env
            .iter()
            .any(|(key, value)| key == "IS_SANDBOX" && value == "1");
        if should_set_claude_sandbox_env(std::env::consts::OS) {
            assert!(has_sandbox);
        } else {
            assert!(!has_sandbox);
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
    fn test_is_fast_exit_threshold() {
        assert!(is_fast_exit(0));
        assert!(is_fast_exit(FAST_EXIT_THRESHOLD_MS - 1));
        assert!(!is_fast_exit(FAST_EXIT_THRESHOLD_MS));
    }

    #[test]
    fn test_apply_pty_wrapper_macos_option_terminator() {
        let args = vec![
            "resume".to_string(),
            "--resume".to_string(),
            "-c".to_string(),
            "value".to_string(),
        ];
        let (exe, wrapped) = apply_pty_wrapper("codex", &args);

        #[cfg(target_os = "macos")]
        {
            assert_eq!(exe, "script");
            assert_eq!(wrapped.first().map(String::as_str), Some("-q"));
            assert_eq!(wrapped.get(1).map(String::as_str), Some("/dev/null"));
            assert_eq!(wrapped.get(2).map(String::as_str), Some("codex"));
            assert_eq!(wrapped[3..], args[..]);
        }

        #[cfg(not(target_os = "macos"))]
        {
            assert_eq!(exe, "codex");
            assert_eq!(wrapped, args);
        }
    }

    #[test]
    fn test_build_launching_message() {
        let config = sample_config(CodingAgent::CodexCli);
        assert_eq!(build_launching_message(&config), "Launching Codex...");
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
        let env_vars = vec![
            ("API_KEY".to_string(), "secret".to_string()),
            ("DEBUG".to_string(), "true".to_string()),
        ];
        let lines = build_launch_log_lines(
            &config,
            &agent_args,
            "latest",
            &ExecutionMethod::Runner {
                label: "bunx".to_string(),
                package_spec: "@openai/codex@latest".to_string(),
            },
            &env_vars,
        );
        assert!(lines.contains(&"Working directory: /tmp/worktree".to_string()));
        assert!(lines.contains(&"Model: gpt-5.2-codex".to_string()));
        assert!(lines.contains(&"Reasoning: high".to_string()));
        assert!(lines.contains(&"Mode: Continue session".to_string()));
        assert!(lines.contains(&"Skip permissions: enabled".to_string()));
        assert!(lines.contains(&"Env:".to_string()));
        assert!(lines.contains(&"  API_KEY=secret".to_string()));
        assert!(lines.contains(&"  DEBUG=true".to_string()));
        assert!(lines.contains(&"Version: latest".to_string()));
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
        let env_vars = Vec::new();
        let lines = build_launch_log_lines(
            &config,
            &agent_args,
            "installed",
            &ExecutionMethod::Installed {
                command: "claude".to_string(),
            },
            &env_vars,
        );
        assert!(lines.contains(&"Model: sonnet".to_string()));
        assert!(!lines.iter().any(|line| line.starts_with("Reasoning: ")));
        assert!(lines.contains(&"Env: (none)".to_string()));
        assert!(lines.contains(&"Using locally installed claude".to_string()));
    }

    #[test]
    fn test_session_updater_stop_is_fast() {
        let temp = TempDir::new().unwrap();
        let context = sample_update_context(temp.path().to_path_buf());
        let updater = spawn_session_updater(context, Duration::from_secs(30));
        let started = Instant::now();
        updater.stop();
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn test_session_update_context_sets_collaboration_modes() {
        let temp = TempDir::new().unwrap();
        let context = sample_update_context(temp.path().to_path_buf());
        let entry = context.to_entry();
        assert_eq!(entry.collaboration_modes, Some(true));
    }

    #[test]
    fn test_should_warn_skip_install_detects_missing_node_modules() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("package.json"), "{}").unwrap();

        let pm = should_warn_skip_install(temp.path());
        assert_eq!(pm, Some("npm"));
    }

    #[test]
    fn test_should_warn_skip_install_returns_none_when_installed() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("package.json"), "{}").unwrap();
        fs::create_dir_all(temp.path().join("node_modules")).unwrap();

        let pm = should_warn_skip_install(temp.path());
        assert!(pm.is_none());
    }

    #[test]
    fn test_skip_install_warning_message_formats() {
        let message = skip_install_warning_message("npm");
        assert_eq!(
            message,
            "Auto install disabled. Skipping dependency install. Run \"npm install\" if needed or set GWT_AGENT_AUTO_INSTALL_DEPS=true."
        );
    }

    #[test]
    fn test_prepare_launch_plan_progress_order() {
        let temp = TempDir::new().unwrap();
        fs::write(temp.path().join("package.json"), "{}").unwrap();
        let mut config = sample_config(CodingAgent::CodexCli);
        config.worktree_path = temp.path().to_path_buf();
        config.auto_install_deps = true;
        let mut steps = Vec::new();

        let plan = prepare_launch_plan(config, |step| steps.push(step)).unwrap();

        assert_eq!(
            steps,
            vec![
                LaunchProgress::BuildingCommand,
                LaunchProgress::CheckingDependencies,
                LaunchProgress::InstallingDependencies {
                    manager: "npm".to_string()
                },
            ]
        );
        assert_eq!(
            plan.install_plan,
            InstallPlan::Install {
                manager: "npm".to_string()
            }
        );
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
    fn test_normalize_session_id_for_launch_keeps_existing_session() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();
        let sessions_dir = home.join(".codex").join("sessions").join("2026");
        fs::create_dir_all(&sessions_dir).unwrap();
        fs::write(sessions_dir.join("rollout-sess-123.jsonl"), "{}").unwrap();

        let mut config = sample_config(CodingAgent::CodexCli);
        config.execution_mode = ExecutionMode::Resume;
        config.session_id = Some("sess-123".to_string());

        let (normalized, warning) =
            normalize_session_id_for_launch_with_home(config, Some(home.to_path_buf()));

        assert_eq!(normalized.session_id.as_deref(), Some("sess-123"));
        assert!(warning.is_none());
    }

    #[test]
    fn test_normalize_session_id_for_launch_drops_missing_session() {
        let temp = TempDir::new().unwrap();
        let home = temp.path();

        let mut config = sample_config(CodingAgent::CodexCli);
        config.execution_mode = ExecutionMode::Continue;
        config.session_id = Some("missing".to_string());

        let (normalized, warning) =
            normalize_session_id_for_launch_with_home(config, Some(home.to_path_buf()));

        assert!(normalized.session_id.is_none());
        assert!(warning.is_some());
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

    // SPEC-861d8cdf T-101 tests

    #[test]
    fn test_hook_user_prompt_submit_sets_running() {
        let payload = serde_json::json!({});
        let status = hook_event_to_status("UserPromptSubmit", &payload);
        assert_eq!(status, AgentStatus::Running);
    }

    #[test]
    fn test_hook_pre_tool_use_sets_running() {
        let payload = serde_json::json!({});
        let status = hook_event_to_status("PreToolUse", &payload);
        assert_eq!(status, AgentStatus::Running);
    }

    #[test]
    fn test_hook_post_tool_use_sets_running() {
        let payload = serde_json::json!({});
        let status = hook_event_to_status("PostToolUse", &payload);
        assert_eq!(status, AgentStatus::Running);
    }

    #[test]
    fn test_hook_stop_sets_stopped() {
        let payload = serde_json::json!({});
        let status = hook_event_to_status("Stop", &payload);
        assert_eq!(status, AgentStatus::Stopped);
    }

    #[test]
    fn test_hook_subagent_stop_sets_stopped() {
        let payload = serde_json::json!({});
        let status = hook_event_to_status("SubagentStop", &payload);
        assert_eq!(status, AgentStatus::Stopped);
    }

    #[test]
    fn test_hook_notification_permission_prompt_sets_waiting_input() {
        let payload = serde_json::json!({
            "notification": {
                "type": "permission_prompt"
            }
        });
        let status = hook_event_to_status("Notification", &payload);
        assert_eq!(status, AgentStatus::WaitingInput);
    }

    #[test]
    fn test_hook_notification_other_type_sets_running() {
        let payload = serde_json::json!({
            "notification": {
                "type": "info"
            }
        });
        let status = hook_event_to_status("Notification", &payload);
        assert_eq!(status, AgentStatus::Running);
    }

    #[test]
    fn test_hook_session_start_sets_running() {
        let payload = serde_json::json!({});
        let status = hook_event_to_status("SessionStart", &payload);
        assert_eq!(status, AgentStatus::Running);
    }

    #[test]
    fn test_hook_session_end_sets_stopped() {
        let payload = serde_json::json!({});
        let status = hook_event_to_status("SessionEnd", &payload);
        assert_eq!(status, AgentStatus::Stopped);
    }

    #[test]
    fn test_hook_unknown_event_sets_running() {
        let payload = serde_json::json!({});
        let status = hook_event_to_status("UnknownEvent", &payload);
        assert_eq!(status, AgentStatus::Running);
    }

    #[test]
    fn test_hook_event_case_insensitive() {
        let payload = serde_json::json!({});
        // Test lowercase
        assert_eq!(
            hook_event_to_status("userpromptsubmit", &payload),
            AgentStatus::Running
        );
        // Test uppercase
        assert_eq!(hook_event_to_status("STOP", &payload), AgentStatus::Stopped);
        // Test mixed case
        assert_eq!(
            hook_event_to_status("SessionStart", &payload),
            AgentStatus::Running
        );
    }

    // ============================================
    // Progress Modal Tests (T001-T005)
    // ============================================

    #[test]
    fn test_progress_step_kind_all_returns_6_steps() {
        let all = ProgressStepKind::all();
        assert_eq!(all.len(), 6);
        assert_eq!(all[0], ProgressStepKind::FetchRemote);
        assert_eq!(all[5], ProgressStepKind::CheckDependencies);
    }

    #[test]
    fn test_progress_step_kind_messages() {
        assert_eq!(
            ProgressStepKind::FetchRemote.message(),
            "Fetching remote..."
        );
        assert_eq!(
            ProgressStepKind::ValidateBranch.message(),
            "Validating branch..."
        );
        assert_eq!(
            ProgressStepKind::GeneratePath.message(),
            "Generating path..."
        );
        assert_eq!(
            ProgressStepKind::CheckConflicts.message(),
            "Checking conflicts..."
        );
        assert_eq!(
            ProgressStepKind::CreateWorktree.message(),
            "Creating worktree..."
        );
        assert_eq!(
            ProgressStepKind::CheckDependencies.message(),
            "Checking dependencies..."
        );
    }

    #[test]
    fn test_step_status_default_is_pending() {
        let status: StepStatus = Default::default();
        assert_eq!(status, StepStatus::Pending);
    }

    #[test]
    fn test_progress_step_new_is_pending() {
        let step = ProgressStep::new(ProgressStepKind::FetchRemote);
        assert_eq!(step.kind, ProgressStepKind::FetchRemote);
        assert_eq!(step.status, StepStatus::Pending);
        assert!(step.started_at.is_none());
        assert!(step.error_message.is_none());
    }

    #[test]
    fn test_progress_step_start_sets_running() {
        let mut step = ProgressStep::new(ProgressStepKind::FetchRemote);
        step.start();
        assert_eq!(step.status, StepStatus::Running);
        assert!(step.started_at.is_some());
    }

    #[test]
    fn test_progress_step_complete_sets_completed() {
        let mut step = ProgressStep::new(ProgressStepKind::FetchRemote);
        step.start();
        step.complete();
        assert_eq!(step.status, StepStatus::Completed);
    }

    #[test]
    fn test_progress_step_fail_sets_failed_with_message() {
        let mut step = ProgressStep::new(ProgressStepKind::FetchRemote);
        step.start();
        step.fail("Network error".to_string());
        assert_eq!(step.status, StepStatus::Failed);
        assert_eq!(step.error_message, Some("Network error".to_string()));
    }

    #[test]
    fn test_progress_step_skip_sets_skipped() {
        let mut step = ProgressStep::new(ProgressStepKind::FetchRemote);
        step.skip();
        assert_eq!(step.status, StepStatus::Skipped);
    }

    #[test]
    fn test_progress_step_markers() {
        let mut step = ProgressStep::new(ProgressStepKind::FetchRemote);
        assert_eq!(step.marker(), "[ ]"); // Pending

        step.start();
        assert_eq!(step.marker(), "[>]"); // Running

        step.complete();
        assert_eq!(step.marker(), "[x]"); // Completed

        let mut step2 = ProgressStep::new(ProgressStepKind::FetchRemote);
        step2.start();
        step2.fail("error".to_string());
        assert_eq!(step2.marker(), "[!]"); // Failed

        let mut step3 = ProgressStep::new(ProgressStepKind::FetchRemote);
        step3.skip();
        assert_eq!(step3.marker(), "[skip]"); // Skipped
    }

    #[test]
    fn test_progress_step_elapsed_secs_none_if_not_started() {
        let step = ProgressStep::new(ProgressStepKind::FetchRemote);
        assert!(step.elapsed_secs().is_none());
    }

    #[test]
    fn test_progress_step_elapsed_secs_some_if_started() {
        let mut step = ProgressStep::new(ProgressStepKind::FetchRemote);
        step.start();
        let elapsed = step.elapsed_secs();
        assert!(elapsed.is_some());
        assert!(elapsed.unwrap() >= 0.0);
    }

    #[test]
    fn test_progress_step_should_show_elapsed_false_under_3_secs() {
        let mut step = ProgressStep::new(ProgressStepKind::FetchRemote);
        step.start();
        // Just started, should be under 3 seconds
        assert!(!step.should_show_elapsed());
    }
}
