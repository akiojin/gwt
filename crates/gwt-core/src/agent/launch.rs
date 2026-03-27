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
            default_args: &[],
            color: AgentColor::Green,
        },
        AgentDef {
            id: "codex",
            display_name: "Codex CLI",
            command: "codex",
            default_args: &[],
            color: AgentColor::Blue,
        },
        AgentDef {
            id: "gemini",
            display_name: "Gemini CLI",
            command: "gemini",
            default_args: &[],
            color: AgentColor::Cyan,
        },
        AgentDef {
            id: "opencode",
            display_name: "OpenCode",
            command: "opencode",
            default_args: &[],
            color: AgentColor::Yellow,
        },
        AgentDef {
            id: "copilot",
            display_name: "GitHub Copilot",
            command: "copilot",
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
// AgentLaunchBuilder
// ---------------------------------------------------------------------------

/// Builder for constructing a [`BuiltinLaunchConfig`] for a coding agent.
pub struct AgentLaunchBuilder {
    agent_id: String,
    working_dir: PathBuf,
    branch_name: String,
    env_overrides: HashMap<String, String>,
    interactive: bool,
}

impl AgentLaunchBuilder {
    /// Create a new builder for the given *agent_id* and *working_dir*.
    pub fn new(agent_id: impl Into<String>, working_dir: impl Into<PathBuf>) -> Self {
        Self {
            agent_id: agent_id.into(),
            working_dir: working_dir.into(),
            branch_name: String::new(),
            env_overrides: HashMap::new(),
            interactive: false,
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

    /// Mark the launch as interactive (e.g. shell spawn).
    pub fn interactive(mut self, interactive: bool) -> Self {
        self.interactive = interactive;
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

        // Resolve the command via PATH (fall back to bare name so the PTY
        // layer can report a proper "command not found").
        let command = which::which(def.command)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| def.command.to_string());

        let args: Vec<String> = def.default_args.iter().map(|s| s.to_string()).collect();

        let mut env_vars = self.env_overrides;
        ensure_terminal_env_defaults(&mut env_vars);

        Ok(BuiltinLaunchConfig {
            command,
            args,
            working_dir: self.working_dir,
            branch_name: self.branch_name,
            agent_name: def.display_name.to_string(),
            agent_color: def.color,
            env_vars,
            terminal_shell: None,
            interactive: self.interactive,
            windows_force_utf8: false,
        })
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
    fn test_resolve_shell_command() {
        let (cmd, _args) = resolve_shell_command();
        assert!(!cmd.is_empty());
        #[cfg(not(windows))]
        {
            // On Unix the result should be a path
            assert!(cmd.starts_with('/') || cmd.contains("sh"));
        }
    }
}
