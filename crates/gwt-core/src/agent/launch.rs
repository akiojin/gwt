//! Agent Launch Builder
//!
//! Reusable module for constructing [`BuiltinLaunchConfig`] for different agent
//! types (Claude Code, Codex CLI, Gemini CLI, etc.) and plain shell launches.
//!
//! Extracted from `gwt-tauri` terminal commands so that both Tauri and future
//! TUI backends can share the same launch-parameter logic.

use std::{collections::HashMap, path::PathBuf};

use crate::terminal::{AgentColor, BuiltinLaunchConfig};

// ---------------------------------------------------------------------------
// AgentDef — static metadata for built-in agents
// ---------------------------------------------------------------------------

/// Metadata about a built-in coding agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentDef {
    /// Machine-readable identifier (e.g. `"claude"`).
    pub id: &'static str,
    /// Human-readable display name (e.g. `"Claude Code"`).
    pub display_name: &'static str,
    /// CLI command name (e.g. `"claude"`).
    pub command: &'static str,
    /// npm/bunx package name for fallback installation (e.g. `"@anthropic-ai/claude-code"`).
    pub bunx_package: &'static str,
    /// Default CLI arguments appended to every launch.
    pub default_args: &'static [&'static str],
    /// Default tab/header color.
    pub color: AgentColor,
}

/// Returns the list of all known built-in agents.
pub fn builtin_agent_defs() -> &'static [AgentDef] {
    static DEFS: &[AgentDef] = &[
        AgentDef {
            id: "claude",
            display_name: "Claude Code",
            command: "claude",
            bunx_package: "@anthropic-ai/claude-code",
            default_args: &[],
            color: AgentColor::Green,
        },
        AgentDef {
            id: "codex",
            display_name: "Codex CLI",
            command: "codex",
            bunx_package: "@openai/codex",
            default_args: &[],
            color: AgentColor::Blue,
        },
        AgentDef {
            id: "gemini",
            display_name: "Gemini CLI",
            command: "gemini",
            bunx_package: "@google/gemini-cli",
            default_args: &[],
            color: AgentColor::Cyan,
        },
        AgentDef {
            id: "opencode",
            display_name: "OpenCode",
            command: "opencode",
            bunx_package: "opencode-ai",
            default_args: &[],
            color: AgentColor::Yellow,
        },
        AgentDef {
            id: "copilot",
            display_name: "GitHub Copilot",
            command: "copilot",
            bunx_package: "@github/copilot",
            default_args: &[],
            color: AgentColor::Magenta,
        },
    ];
    DEFS
}

/// Look up an [`AgentDef`] by its identifier.
pub fn find_agent_def(agent_id: &str) -> Option<&'static AgentDef> {
    builtin_agent_defs().iter().find(|d| d.id == agent_id)
}

/// Returns the default [`AgentColor`] for the given agent identifier.
///
/// Falls back to [`AgentColor::White`] for unknown agents.
pub fn agent_color_for(agent_id: &str) -> AgentColor {
    find_agent_def(agent_id)
        .map(|d| d.color)
        .unwrap_or(AgentColor::White)
}

// ---------------------------------------------------------------------------
// Environment variable helpers
// ---------------------------------------------------------------------------

/// Ensures `TERM` and `COLORTERM` are present in *env*, setting sensible
/// defaults (`xterm-256color` / `truecolor`) when missing.
pub fn ensure_terminal_env_defaults(env: &mut HashMap<String, String>) {
    env.entry("TERM".to_string())
        .or_insert_with(|| "xterm-256color".to_string());
    env.entry("COLORTERM".to_string())
        .or_insert_with(|| "truecolor".to_string());
}

/// Merge two environment maps.  Values in *overrides* take priority over
/// *base*.
pub fn merge_env_vars(
    base: &HashMap<String, String>,
    overrides: &HashMap<String, String>,
) -> HashMap<String, String> {
    let mut merged = base.clone();
    merged.extend(overrides.iter().map(|(k, v)| (k.clone(), v.clone())));
    merged
}

// ---------------------------------------------------------------------------
// Shell resolution
// ---------------------------------------------------------------------------

/// Detect the user's default shell.
///
/// On macOS / Linux the `SHELL` environment variable is used, falling back to
/// `/bin/sh`.  On Windows, `powershell` is returned.
pub fn resolve_shell_command() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        ("powershell".to_string(), Vec::new())
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        (shell, Vec::new())
    }
}

// ---------------------------------------------------------------------------
// SessionMode
// ---------------------------------------------------------------------------

/// Session mode for agent launch (normal, continue, resume).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum SessionMode {
    /// Start a fresh session.
    #[default]
    Normal,
    /// Continue the most recent session.
    Continue,
    /// Resume a specific (or latest) session.
    Resume,
}

// ---------------------------------------------------------------------------
// AgentLaunchBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing a [`BuiltinLaunchConfig`] for a coding agent.
///
/// Handles environment merging (OS env -> profile env -> overrides),
/// bunx/npx fallback for missing commands, and agent-specific CLI arguments
/// (model, session mode, skip-permissions, etc.).
pub struct AgentLaunchBuilder {
    agent_id: String,
    working_dir: PathBuf,
    branch_name: String,
    env_overrides: HashMap<String, String>,
    os_env: Option<HashMap<String, String>>,
    profile_name: Option<String>,
    model: Option<String>,
    skip_permissions: bool,
    session_mode: SessionMode,
    resume_session_id: Option<String>,
    interactive: bool,
    auto_worktree: bool,
    repo_root: Option<PathBuf>,
    fast_mode: bool,
    reasoning_level: Option<String>,
    agent_version: Option<String>,
    extra_args: Vec<String>,
}

impl AgentLaunchBuilder {
    /// Create a new builder for the given *agent_id* and *working_dir*.
    pub fn new(agent_id: impl Into<String>, working_dir: impl Into<PathBuf>) -> Self {
        Self {
            agent_id: agent_id.into(),
            working_dir: working_dir.into(),
            branch_name: String::new(),
            env_overrides: HashMap::new(),
            os_env: None,
            profile_name: None,
            model: None,
            skip_permissions: false,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            interactive: false,
            auto_worktree: false,
            repo_root: None,
            fast_mode: false,
            reasoning_level: None,
            agent_version: None,
            extra_args: Vec::new(),
        }
    }

    /// Set the branch name shown in the terminal tab.
    pub fn branch_name(mut self, name: impl Into<String>) -> Self {
        self.branch_name = name.into();
        self
    }

    /// Add a single environment variable override.
    pub fn env_var(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_overrides.insert(key.into(), value.into());
        self
    }

    /// Provide the OS environment snapshot (instead of reading `std::env::vars()`).
    pub fn with_os_env(mut self, env: HashMap<String, String>) -> Self {
        self.os_env = Some(env);
        self
    }

    /// Set the profile name to use for environment merging.
    pub fn profile_name(mut self, name: impl Into<String>) -> Self {
        self.profile_name = Some(name.into());
        self
    }

    /// Set the model to pass to the agent CLI.
    pub fn model(mut self, model: Option<&str>) -> Self {
        self.model = model.map(|s| s.to_string());
        self
    }

    /// Enable or disable skip-permissions mode.
    pub fn skip_permissions(mut self, skip: bool) -> Self {
        self.skip_permissions = skip;
        self
    }

    /// Set the session mode (Normal, Continue, Resume).
    pub fn session_mode(mut self, mode: SessionMode) -> Self {
        self.session_mode = mode;
        self
    }

    /// Set the session ID for resume mode.
    pub fn resume_session_id(mut self, id: impl Into<String>) -> Self {
        self.resume_session_id = Some(id.into());
        self
    }

    /// Mark the launch as interactive (e.g. shell spawn).
    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
        self
    }

    /// Enable automatic worktree creation for the branch.
    pub fn auto_worktree(mut self, auto: bool) -> Self {
        self.auto_worktree = auto;
        self
    }

    /// Set the repository root path (used for worktree creation and GWT_PROJECT_ROOT).
    pub fn repo_root(mut self, path: &std::path::Path) -> Self {
        self.repo_root = Some(path.to_path_buf());
        self
    }

    /// Enable or disable Codex fast mode (service_tier=fast).
    pub fn fast_mode(mut self, fast: bool) -> Self {
        self.fast_mode = fast;
        self
    }

    /// Set the reasoning level for Codex (`low`, `medium`, `high`, `xhigh`).
    pub fn reasoning_level(mut self, level: Option<&str>) -> Self {
        self.reasoning_level = level.map(|s| s.to_string());
        self
    }

    /// Set the agent version (`None` = installed, `Some("latest")`, `Some("1.2.3")`).
    pub fn agent_version(mut self, version: Option<&str>) -> Self {
        self.agent_version = version.map(|s| s.to_string());
        self
    }

    /// Append extra CLI arguments to the agent command.
    pub fn extra_args(mut self, args: Vec<String>) -> Self {
        self.extra_args = args;
        self
    }

    /// Build the final [`BuiltinLaunchConfig`].
    ///
    /// Returns [`GwtError::AgentNotFound`] when the *agent_id* is unknown.
    pub fn build(self) -> crate::error::Result<BuiltinLaunchConfig> {
        let def = find_agent_def(&self.agent_id).ok_or_else(|| {
            crate::error::GwtError::AgentNotFound {
                name: self.agent_id.clone(),
            }
        })?;

        let mut env_vars = self.collect_env_vars();
        let (command, mut args) = self.resolve_command_and_args(def);
        self.append_agent_args(def, &mut args);

        // Append user-supplied extra arguments
        args.extend(self.extra_args.iter().cloned());

        // Agent-specific environment variables
        if self.agent_id == "claude" {
            env_vars
                .entry("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".to_string())
                .or_insert_with(|| "1".to_string());
        }
        if let Some(ref root) = self.repo_root {
            env_vars.insert(
                "GWT_PROJECT_ROOT".to_string(),
                root.to_string_lossy().to_string(),
            );
        }
        ensure_terminal_env_defaults(&mut env_vars);

        // Worktree auto-creation
        let final_working_dir = self.resolve_working_dir();

        Ok(BuiltinLaunchConfig {
            command,
            args,
            working_dir: final_working_dir,
            branch_name: self.branch_name,
            agent_name: def.display_name.to_string(),
            agent_color: def.color,
            env_vars,
            terminal_shell: None,
            interactive: self.interactive,
            windows_force_utf8: false,
        })
    }

    // --- Private helpers ---

    /// Collect and merge environment variables: OS env -> profile env -> overrides.
    fn collect_env_vars(&self) -> HashMap<String, String> {
        let mut env_vars = self
            .os_env
            .clone()
            .unwrap_or_else(|| std::env::vars().collect());

        // Merge profile env if available
        if let Ok(profiles_config) = crate::config::ProfilesConfig::load() {
            let profile_name = self
                .profile_name
                .as_deref()
                .or(profiles_config.active.as_deref());
            if let Some(name) = profile_name {
                if let Some(profile) = profiles_config.profiles.get(name) {
                    // Remove disabled env vars
                    for key in &profile.disabled_env {
                        env_vars.remove(key);
                    }
                    // Override with profile env vars
                    for (k, v) in &profile.env {
                        env_vars.insert(k.clone(), v.clone());
                    }
                    // Inject AI API key if missing or empty
                    let needs_key = !env_vars.contains_key("OPENAI_API_KEY")
                        || env_vars
                            .get("OPENAI_API_KEY")
                            .map(|v| v.trim().is_empty())
                            .unwrap_or(true);
                    if needs_key {
                        if let Some(ai) = &profile.ai {
                            let key = ai.api_key.trim();
                            if !key.is_empty() {
                                env_vars.insert("OPENAI_API_KEY".to_string(), key.to_string());
                            }
                        }
                    }
                }
            }
        }

        // Apply env overrides (highest priority)
        for (k, v) in &self.env_overrides {
            env_vars.insert(k.clone(), v.clone());
        }

        env_vars
    }

    /// Resolve command with bunx/npx fallback and agent_version support.
    fn resolve_command_and_args(&self, def: &AgentDef) -> (String, Vec<String>) {
        // If agent_version is set to a non-installed value, use bunx/npx
        if let Some(ref ver) = self.agent_version {
            if !ver.is_empty() && ver != "installed" {
                let runner = if which::which("bunx").is_ok() {
                    "bunx"
                } else if which::which("npx").is_ok() {
                    "npx"
                } else {
                    "bunx"
                };
                let pkg = if ver == "latest" {
                    def.bunx_package.to_string()
                } else {
                    format!("{}@{}", def.bunx_package, ver)
                };
                return (runner.to_string(), vec![pkg]);
            }
        }

        // Default: resolve installed command with bunx/npx fallback
        let command = which::which(def.command)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| {
                if which::which("bunx").is_ok() {
                    "bunx".to_string()
                } else if which::which("npx").is_ok() {
                    "npx".to_string()
                } else {
                    def.command.to_string()
                }
            });

        let args: Vec<String> = if command.ends_with("bunx") || command.ends_with("npx") {
            vec![def.bunx_package.to_string()]
        } else {
            def.default_args.iter().map(|s| s.to_string()).collect()
        };

        (command, args)
    }

    /// Append agent-specific CLI arguments (model, session mode, permissions).
    fn append_agent_args(&self, _def: &AgentDef, args: &mut Vec<String>) {
        match self.agent_id.as_str() {
            "claude" => {
                match self.session_mode {
                    SessionMode::Continue => {
                        if let Some(ref id) = self.resume_session_id {
                            args.push("--resume".to_string());
                            args.push(id.clone());
                        } else {
                            args.push("--continue".to_string());
                        }
                    }
                    SessionMode::Resume => {
                        args.push("--resume".to_string());
                        if let Some(ref id) = self.resume_session_id {
                            args.push(id.clone());
                        }
                    }
                    SessionMode::Normal => {}
                }
                if self.skip_permissions {
                    args.push("--dangerously-skip-permissions".to_string());
                }
                if let Some(ref model) = self.model {
                    args.push("--model".to_string());
                    args.push(model.clone());
                }
            }
            "codex" => {
                match self.session_mode {
                    SessionMode::Continue | SessionMode::Resume => {
                        args.insert(0, "resume".to_string());
                        if let Some(ref id) = self.resume_session_id {
                            args.insert(1, id.clone());
                        } else if self.session_mode == SessionMode::Continue {
                            args.insert(1, "--last".to_string());
                        }
                    }
                    SessionMode::Normal => {}
                }

                // Delegate model/reasoning/fast/sandbox to codex_default_args
                args.extend(crate::agent::codex::codex_default_args(
                    self.model.as_deref(),
                    self.reasoning_level.as_deref(),
                    None,
                    self.skip_permissions,
                    self.fast_mode,
                    true,
                    false,
                ));

                if self.skip_permissions {
                    args.push(crate::agent::codex::codex_skip_permissions_flag(None).to_string());
                }
            }
            "gemini" => {
                match self.session_mode {
                    SessionMode::Continue | SessionMode::Resume => {
                        args.push("-r".to_string());
                        if let Some(ref id) = self.resume_session_id {
                            args.push(id.clone());
                        } else {
                            args.push("latest".to_string());
                        }
                    }
                    SessionMode::Normal => {}
                }
                if self.skip_permissions {
                    args.push("-y".to_string());
                }
                if let Some(ref model) = self.model {
                    args.push("-m".to_string());
                    args.push(model.clone());
                }
            }
            "opencode" => {
                if let Some(ref model) = self.model {
                    args.push("-m".to_string());
                    args.push(model.clone());
                }
            }
            "copilot" => {
                if self.skip_permissions {
                    args.push("--allow-all-tools".to_string());
                }
                if let Some(ref model) = self.model {
                    args.push("--model".to_string());
                    args.push(model.clone());
                }
            }
            _ => {}
        }
    }

    /// Resolve the working directory, creating a worktree if needed.
    fn resolve_working_dir(&self) -> PathBuf {
        if self.auto_worktree && !self.branch_name.is_empty() {
            if let Some(ref repo_root) = self.repo_root {
                if let Ok(wt_manager) = crate::worktree::WorktreeManager::new(repo_root) {
                    match wt_manager.get_by_branch(&self.branch_name) {
                        Ok(Some(wt)) => return wt.path,
                        Ok(None) => {
                            if let Ok(wt) = wt_manager.create_for_branch(&self.branch_name) {
                                return wt.path;
                            }
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        self.working_dir.clone()
    }
}

// ---------------------------------------------------------------------------
// ShellLaunchBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing a [`BuiltinLaunchConfig`] for a plain shell.
pub struct ShellLaunchBuilder {
    working_dir: PathBuf,
    shell_override: Option<String>,
}

impl ShellLaunchBuilder {
    /// Create a new builder for a shell launch in *working_dir*.
    pub fn new(working_dir: impl Into<PathBuf>) -> Self {
        Self {
            working_dir: working_dir.into(),
            shell_override: None,
        }
    }

    /// Override the detected shell with a specific command.
    pub fn shell(mut self, shell: impl Into<String>) -> Self {
        self.shell_override = Some(shell.into());
        self
    }

    /// Build the final [`BuiltinLaunchConfig`].
    pub fn build(self) -> BuiltinLaunchConfig {
        let (command, args) = if let Some(ref shell) = self.shell_override {
            (shell.clone(), Vec::new())
        } else {
            resolve_shell_command()
        };

        let mut env_vars = HashMap::new();
        ensure_terminal_env_defaults(&mut env_vars);

        BuiltinLaunchConfig {
            command,
            args,
            working_dir: self.working_dir,
            branch_name: String::new(),
            agent_name: "Shell".to_string(),
            agent_color: AgentColor::White,
            env_vars,
            terminal_shell: self.shell_override,
            interactive: true,
            windows_force_utf8: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_agent_defs_contains_all_known() {
        let defs = builtin_agent_defs();
        let ids: Vec<&str> = defs.iter().map(|d| d.id).collect();
        assert!(ids.contains(&"claude"));
        assert!(ids.contains(&"codex"));
        assert!(ids.contains(&"gemini"));
        assert!(ids.contains(&"opencode"));
        assert!(ids.contains(&"copilot"));
    }

    #[test]
    fn test_agent_color_for_claude_is_green() {
        assert_eq!(agent_color_for("claude"), AgentColor::Green);
    }

    #[test]
    fn test_agent_color_for_unknown_is_white() {
        assert_eq!(agent_color_for("unknown-agent"), AgentColor::White);
    }

    #[test]
    fn test_ensure_terminal_env_defaults_sets_term() {
        let mut env = HashMap::new();
        ensure_terminal_env_defaults(&mut env);
        assert_eq!(env.get("TERM").unwrap(), "xterm-256color");
        assert_eq!(env.get("COLORTERM").unwrap(), "truecolor");
    }

    #[test]
    fn test_ensure_terminal_env_defaults_preserves_existing() {
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "screen".to_string());
        ensure_terminal_env_defaults(&mut env);
        assert_eq!(env.get("TERM").unwrap(), "screen");
        assert_eq!(env.get("COLORTERM").unwrap(), "truecolor");
    }

    #[test]
    fn test_merge_env_vars_override_wins() {
        let mut base = HashMap::new();
        base.insert("A".to_string(), "1".to_string());
        base.insert("B".to_string(), "2".to_string());

        let mut overrides = HashMap::new();
        overrides.insert("B".to_string(), "override".to_string());
        overrides.insert("C".to_string(), "3".to_string());

        let merged = merge_env_vars(&base, &overrides);
        assert_eq!(merged.get("A").unwrap(), "1");
        assert_eq!(merged.get("B").unwrap(), "override");
        assert_eq!(merged.get("C").unwrap(), "3");
    }

    #[test]
    fn test_shell_launch_builder_default() {
        let config = ShellLaunchBuilder::new("/tmp/test").build();
        assert_eq!(config.working_dir, PathBuf::from("/tmp/test"));
        assert!(config.interactive);
        assert_eq!(config.agent_name, "Shell");
        assert_eq!(config.agent_color, AgentColor::White);
        assert!(!config.command.is_empty());
        assert!(config.env_vars.contains_key("TERM"));
    }

    #[test]
    fn test_shell_launch_builder_custom_shell() {
        let config = ShellLaunchBuilder::new("/tmp/test")
            .shell("/usr/bin/fish")
            .build();
        assert_eq!(config.command, "/usr/bin/fish");
        assert_eq!(config.terminal_shell, Some("/usr/bin/fish".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_basic() {
        // "claude" may or may not be installed, but the builder should still
        // produce a valid config (falling back to bare command name).
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .branch_name("feature/test")
            .env_var("MY_VAR", "hello")
            .interactive(false)
            .build()
            .expect("build should succeed for known agent");

        assert_eq!(config.working_dir, PathBuf::from("/tmp/work"));
        assert_eq!(config.branch_name, "feature/test");
        assert_eq!(config.agent_name, "Claude Code");
        assert_eq!(config.agent_color, AgentColor::Green);
        assert_eq!(config.env_vars.get("MY_VAR").unwrap(), "hello");
        assert!(config.env_vars.contains_key("TERM"));
        assert!(!config.interactive);
    }

    #[test]
    fn test_agent_launch_builder_unknown_agent() {
        let result = AgentLaunchBuilder::new("nonexistent", "/tmp").build();
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_launch_builder_with_model() {
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .model(Some("opus"))
            .build()
            .expect("build should succeed");
        assert!(config.args.contains(&"--model".to_string()));
        assert!(config.args.contains(&"opus".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_claude_skip_permissions() {
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .skip_permissions(true)
            .build()
            .expect("build should succeed");
        assert!(config
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_claude_continue_mode() {
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .session_mode(SessionMode::Continue)
            .build()
            .expect("build should succeed");
        assert!(config.args.contains(&"--continue".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_claude_resume_with_id() {
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .session_mode(SessionMode::Resume)
            .resume_session_id("abc123")
            .build()
            .expect("build should succeed");
        assert!(config.args.contains(&"--resume".to_string()));
        assert!(config.args.contains(&"abc123".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_codex_model() {
        let config = AgentLaunchBuilder::new("codex", "/tmp/work")
            .model(Some("o3"))
            .build()
            .expect("build should succeed");
        // codex_default_args uses --model= format
        assert!(config.args.contains(&"--model=o3".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_codex_skip_permissions() {
        let config = AgentLaunchBuilder::new("codex", "/tmp/work")
            .skip_permissions(true)
            .build()
            .expect("build should succeed");
        // Now delegated to codex_skip_permissions_flag()
        assert!(config
            .args
            .contains(&"--dangerously-bypass-approvals-and-sandbox".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_gemini_model() {
        let config = AgentLaunchBuilder::new("gemini", "/tmp/work")
            .model(Some("gemini-2.5-pro"))
            .build()
            .expect("build should succeed");
        assert!(config.args.contains(&"-m".to_string()));
        assert!(config.args.contains(&"gemini-2.5-pro".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_gemini_skip_permissions() {
        let config = AgentLaunchBuilder::new("gemini", "/tmp/work")
            .skip_permissions(true)
            .build()
            .expect("build should succeed");
        assert!(config.args.contains(&"-y".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_copilot_model() {
        let config = AgentLaunchBuilder::new("copilot", "/tmp/work")
            .model(Some("gpt-4o"))
            .build()
            .expect("build should succeed");
        assert!(config.args.contains(&"--model".to_string()));
        assert!(config.args.contains(&"gpt-4o".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_copilot_skip_permissions() {
        let config = AgentLaunchBuilder::new("copilot", "/tmp/work")
            .skip_permissions(true)
            .build()
            .expect("build should succeed");
        assert!(config.args.contains(&"--allow-all-tools".to_string()));
    }

    #[test]
    fn test_agent_launch_builder_claude_sets_agent_teams_env() {
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .with_os_env(HashMap::new())
            .build()
            .expect("build should succeed");
        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS")
                .map(|s| s.as_str()),
            Some("1")
        );
    }

    #[test]
    fn test_agent_launch_builder_repo_root_sets_env() {
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .with_os_env(HashMap::new())
            .repo_root(std::path::Path::new("/my/repo"))
            .build()
            .expect("build should succeed");
        assert_eq!(
            config.env_vars.get("GWT_PROJECT_ROOT").map(|s| s.as_str()),
            Some("/my/repo")
        );
    }

    #[test]
    fn test_session_mode_default_is_normal() {
        assert_eq!(SessionMode::default(), SessionMode::Normal);
    }

    #[test]
    fn test_agent_def_bunx_package() {
        let def = find_agent_def("claude").unwrap();
        assert_eq!(def.bunx_package, "@anthropic-ai/claude-code");
        let def = find_agent_def("codex").unwrap();
        assert_eq!(def.bunx_package, "@openai/codex");
        let def = find_agent_def("gemini").unwrap();
        assert_eq!(def.bunx_package, "@google/gemini-cli");
    }

    #[test]
    fn test_agent_launch_builder_os_env_override() {
        let mut os_env = HashMap::new();
        os_env.insert("MY_KEY".to_string(), "from_os".to_string());
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .with_os_env(os_env)
            .env_var("MY_KEY", "from_override")
            .build()
            .expect("build should succeed");
        // env_var overrides should win over os_env
        assert_eq!(config.env_vars.get("MY_KEY").unwrap(), "from_override");
    }

    #[test]
    fn test_agent_launch_builder_no_model_no_args() {
        let config = AgentLaunchBuilder::new("claude", "/tmp/work")
            .model(None)
            .build()
            .expect("build should succeed");
        assert!(!config.args.contains(&"--model".to_string()));
    }

    #[test]
    fn test_resolve_shell_command() {
        let (cmd, _args) = resolve_shell_command();
        assert!(!cmd.is_empty());
        #[cfg(not(windows))]
        {
            // On Unix the result should be a path
            assert!(cmd.starts_with('/') || cmd.contains("sh"));
        }
    }

    #[test]
    fn test_agent_launch_builder_auto_worktree_fallback_on_invalid_repo() {
        // auto_worktree(true) but repo_root is not a valid git repository;
        // working_dir should fall back to the original value.
        let config = AgentLaunchBuilder::new("claude", "/tmp/not-a-real-repo")
            .branch_name("develop")
            .repo_root(std::path::Path::new("/tmp/not-a-real-repo"))
            .auto_worktree(true)
            .build()
            .expect("build should succeed");
        assert_eq!(config.working_dir, PathBuf::from("/tmp/not-a-real-repo"));
    }

    #[test]
    fn test_agent_launch_builder_without_auto_worktree_keeps_working_dir() {
        // Without auto_worktree, working_dir stays as the initial value
        // even when branch_name and repo_root are set.
        let config = AgentLaunchBuilder::new("claude", "/tmp/original")
            .branch_name("feature/test")
            .repo_root(std::path::Path::new("/tmp/original"))
            .build()
            .expect("build should succeed");
        assert_eq!(config.working_dir, PathBuf::from("/tmp/original"));
    }
}
