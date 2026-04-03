//! Agent launch builder: construct launch configurations for coding agents.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::types::{AgentColor, AgentId, SessionMode};

/// Resolved runner command for agent execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRunner {
    /// Executable to invoke (e.g., "claude", "bunx", "npx").
    pub executable: String,
    /// Base args inserted before agent-specific args (e.g., ["@anthropic-ai/claude-code@1.2.3"]).
    pub base_args: Vec<String>,
}

/// Resolve the runner command based on version selection.
///
/// - `"installed"` → use the agent's direct command (must be in PATH).
/// - `"latest"` or a semver string → use bunx/npx with `@package@version`.
pub fn resolve_runner(agent_id: &AgentId, version: &str) -> ResolvedRunner {
    if version == "installed" || version.is_empty() {
        return ResolvedRunner {
            executable: agent_id.command().to_string(),
            base_args: Vec::new(),
        };
    }

    let Some(package) = agent_id.package_name() else {
        // No npm package — fall back to direct command
        return ResolvedRunner {
            executable: agent_id.command().to_string(),
            base_args: Vec::new(),
        };
    };

    let version_spec = if version == "latest" {
        format!("{}@latest", package)
    } else {
        format!("{}@{}", package, version)
    };

    let (executable, needs_yes) = find_bunx_or_npx();
    let mut base_args = Vec::new();
    if needs_yes {
        base_args.push("--yes".to_string());
    }
    base_args.push(version_spec);

    ResolvedRunner {
        executable,
        base_args,
    }
}

/// Find bunx or npx executable, preferring global bunx over local node_modules.
///
/// Returns `(executable_name, needs_yes_flag)`.
/// - bunx: no `--yes` needed
/// - npx: `--yes` needed to suppress interactive prompt
fn find_bunx_or_npx() -> (String, bool) {
    // Try bunx first, but skip if it's a local node_modules/.bin/bunx
    if let Ok(path) = which::which("bunx") {
        let path_str = path.to_string_lossy();
        if !path_str.contains("node_modules") {
            return (path.to_string_lossy().into_owned(), false);
        }
    }

    // Fall back to npx (needs --yes)
    if let Ok(path) = which::which("npx") {
        return (path.to_string_lossy().into_owned(), true);
    }

    // Last resort: assume bunx is available
    ("bunx".to_string(), false)
}

/// Final configuration used to spawn an agent process.
#[derive(Debug, Clone)]
pub struct LaunchConfig {
    pub command: String,
    pub args: Vec<String>,
    pub env_vars: HashMap<String, String>,
    pub working_dir: Option<PathBuf>,
    pub display_name: String,
    pub color: AgentColor,
}

/// Permission mode for agent launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    Auto,
    DontAsk,
    BypassPermissions,
}

/// Builder for constructing agent launch configurations.
#[derive(Debug, Clone)]
pub struct AgentLaunchBuilder {
    agent_id: AgentId,
    working_dir: Option<PathBuf>,
    branch: Option<String>,
    model: Option<String>,
    version: Option<String>,
    fast_mode: bool,
    reasoning_level: Option<String>,
    session_mode: SessionMode,
    resume_session_id: Option<String>,
    permission_mode: Option<PermissionMode>,
    env_overrides: HashMap<String, String>,
    extra_args: Vec<String>,
}

impl AgentLaunchBuilder {
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            working_dir: None,
            branch: None,
            model: None,
            version: None,
            fast_mode: false,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            permission_mode: None,
            env_overrides: HashMap::new(),
            extra_args: Vec::new(),
        }
    }

    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.working_dir = Some(dir.into());
        self
    }

    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = Some(model.into());
        self
    }

    /// Set the version selection ("installed", "latest", or a semver string).
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    pub fn fast_mode(mut self, enabled: bool) -> Self {
        self.fast_mode = enabled;
        self
    }

    pub fn reasoning_level(mut self, level: impl Into<String>) -> Self {
        self.reasoning_level = Some(level.into());
        self
    }

    pub fn session_mode(mut self, mode: SessionMode) -> Self {
        self.session_mode = mode;
        self
    }

    pub fn resume_session_id(mut self, id: impl Into<String>) -> Self {
        self.resume_session_id = Some(id.into());
        self
    }

    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env_overrides.insert(key.into(), value.into());
        self
    }

    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = Some(mode);
        self
    }

    pub fn extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Build the final `LaunchConfig`.
    pub fn build(self) -> LaunchConfig {
        let mut env_vars = HashMap::new();

        // Common env vars
        env_vars.insert("TERM".to_string(), "xterm-256color".to_string());
        if let Some(ref dir) = self.working_dir {
            env_vars.insert("GWT_PROJECT_ROOT".to_string(), dir.display().to_string());
        }

        // Resolve runner (installed binary vs bunx/npx)
        let runner = resolve_runner(
            &self.agent_id,
            self.version.as_deref().unwrap_or("installed"),
        );

        let mut args = runner.base_args;

        // Agent-specific configuration
        match &self.agent_id {
            AgentId::ClaudeCode => {
                self.build_claude_args(&mut args, &mut env_vars);
            }
            AgentId::Codex => {
                self.build_codex_args(&mut args, &mut env_vars);
            }
            AgentId::Gemini => {
                self.build_gemini_args(&mut args);
            }
            AgentId::OpenCode => {
                self.build_opencode_args(&mut args);
            }
            AgentId::Copilot => {
                self.build_copilot_args(&mut args);
            }
            AgentId::Custom(_) => {
                // No special args for custom agents
            }
        }

        // Extra args at the end
        args.extend(self.extra_args);

        // Apply env overrides last (user wins)
        env_vars.extend(self.env_overrides);

        LaunchConfig {
            command: runner.executable,
            args,
            env_vars,
            working_dir: self.working_dir,
            display_name: self.agent_id.display_name().to_string(),
            color: self.agent_id.default_color(),
        }
    }

    fn build_claude_args(&self, args: &mut Vec<String>, env_vars: &mut HashMap<String, String>) {
        // Claude Code specific env vars
        env_vars.insert("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".into(), "1".into());
        env_vars.insert("CLAUDE_CODE_NO_FLICKER".into(), "1".into());

        // Telemetry/analytics disable
        env_vars.insert("DISABLE_TELEMETRY".into(), "1".into());
        env_vars.insert("DISABLE_ERROR_REPORTING".into(), "1".into());
        env_vars.insert("DISABLE_FEEDBACK_COMMAND".into(), "1".into());
        env_vars.insert("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY".into(), "1".into());
        env_vars.insert(
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
            "1".into(),
        );

        // Permission mode
        if let Some(ref mode) = self.permission_mode {
            args.push("--permission-mode".to_string());
            args.push(
                match mode {
                    PermissionMode::Default => "default",
                    PermissionMode::AcceptEdits => "acceptEdits",
                    PermissionMode::Plan => "plan",
                    PermissionMode::Auto => "auto",
                    PermissionMode::DontAsk => "dontAsk",
                    PermissionMode::BypassPermissions => "bypassPermissions",
                }
                .to_string(),
            );
        }

        // Session mode
        match self.session_mode {
            SessionMode::Continue => args.push("--continue".to_string()),
            SessionMode::Resume => {
                args.push("--resume".to_string());
                if let Some(ref id) = self.resume_session_id {
                    args.push(id.clone());
                }
            }
            SessionMode::Normal => {}
        }

        if let Some(ref model) = self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
    }

    fn build_codex_args(&self, args: &mut Vec<String>, env_vars: &mut HashMap<String, String>) {
        env_vars.insert(
            "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".to_string(),
            "1".to_string(),
        );

        if let Some(ref model) = self.model {
            args.push(format!("--model={}", model));
        }

        // Reasoning level (Codex-specific)
        if let Some(ref level) = self.reasoning_level {
            args.push("-c".to_string());
            args.push(format!("model_reasoning_effort={}", level));
            args.push("-c".to_string());
            args.push("model_reasoning_summaries=detailed".to_string());
        }

        if self.fast_mode {
            args.push("--full-auto".to_string());
        }

        // Version-dependent flags
        let version_str = self.version.as_deref().unwrap_or("");
        let parsed_version = semver::Version::parse(version_str).ok();

        // Web search args
        if let Some(ref ver) = parsed_version {
            if *ver >= semver::Version::new(0, 90, 0) {
                args.push("--enable".to_string());
                args.push("web_search".to_string());
            } else {
                args.push("--enable".to_string());
                args.push("web_search_request".to_string());
            }
        }

        // Sandbox & shell env policies
        args.push("--sandbox".to_string());
        args.push("workspace-write".to_string());
        args.push("-c".to_string());
        args.push("sandbox_workspace_write.network_access=true".to_string());
        args.push("-c".to_string());
        args.push("shell_environment_policy.inherit=all".to_string());
        args.push("-c".to_string());
        args.push("shell_environment_policy.ignore_default_excludes=true".to_string());
        args.push("-c".to_string());
        args.push("shell_environment_policy.experimental_use_profile=true".to_string());
    }

    fn build_gemini_args(&self, args: &mut Vec<String>) {
        if let Some(ref model) = self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
    }

    fn build_opencode_args(&self, _args: &mut Vec<String>) {
        // OpenCode has minimal CLI flags
    }

    fn build_copilot_args(&self, args: &mut Vec<String>) {
        // gh copilot is invoked as `gh copilot`
        args.insert(0, "copilot".to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_default_state() {
        let builder = AgentLaunchBuilder::new(AgentId::ClaudeCode);
        assert_eq!(builder.agent_id, AgentId::ClaudeCode);
        assert!(builder.working_dir.is_none());
        assert!(!builder.fast_mode);
        assert_eq!(builder.session_mode, SessionMode::Normal);
    }

    #[test]
    fn build_claude_normal() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .working_dir("/tmp/project")
            .build();

        assert_eq!(config.command, "claude");
        assert_eq!(config.display_name, "Claude Code");
        assert_eq!(config.color, AgentColor::Green);
        assert_eq!(
            config.env_vars.get("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"),
            Some(&"1".to_string())
        );
        assert_eq!(
            config.env_vars.get("TERM"),
            Some(&"xterm-256color".to_string())
        );
        assert_eq!(
            config.env_vars.get("GWT_PROJECT_ROOT"),
            Some(&"/tmp/project".to_string())
        );
    }

    #[test]
    fn build_claude_continue() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .session_mode(SessionMode::Continue)
            .build();

        assert!(config.args.contains(&"--continue".to_string()));
    }

    #[test]
    fn build_claude_resume_with_id() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .session_mode(SessionMode::Resume)
            .resume_session_id("sess-123")
            .build();

        assert!(config.args.contains(&"--resume".to_string()));
        assert!(config.args.contains(&"sess-123".to_string()));
    }

    #[test]
    fn build_claude_with_model() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .model("claude-sonnet-4-20250514")
            .build();

        assert!(config.args.contains(&"--model".to_string()));
        assert!(config
            .args
            .contains(&"claude-sonnet-4-20250514".to_string()));
    }

    #[test]
    fn build_codex_fast_mode() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .fast_mode(true)
            .build();

        assert_eq!(config.command, "codex");
        assert!(config.args.contains(&"--full-auto".to_string()));
    }

    #[test]
    fn build_codex_with_reasoning_level() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .model("gpt-5.3-codex")
            .reasoning_level("high")
            .build();

        assert!(config.args.contains(&"--model=gpt-5.3-codex".to_string()));
        assert!(config.args.contains(&"-c".to_string()));
        assert!(config
            .args
            .contains(&"model_reasoning_effort=high".to_string()));
        assert!(config
            .args
            .contains(&"model_reasoning_summaries=detailed".to_string()));
    }

    #[test]
    fn build_codex_version_specific_web_search_new() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .version("0.91.0")
            .build();

        assert!(config.args.contains(&"web_search".to_string()));
    }

    #[test]
    fn build_codex_version_specific_web_search_old() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .version("0.80.0")
            .build();

        assert!(config.args.contains(&"web_search_request".to_string()));
    }

    #[test]
    fn build_codex_sandbox_and_shell_policies() {
        let config = AgentLaunchBuilder::new(AgentId::Codex).build();

        assert!(config.args.contains(&"workspace-write".to_string()));
        assert!(config
            .args
            .contains(&"sandbox_workspace_write.network_access=true".to_string()));
        assert!(config
            .args
            .contains(&"shell_environment_policy.inherit=all".to_string()));
    }

    #[test]
    fn build_copilot_prepends_subcommand() {
        let config = AgentLaunchBuilder::new(AgentId::Copilot).build();
        assert_eq!(config.command, "gh");
        assert_eq!(config.args.first(), Some(&"copilot".to_string()));
    }

    #[test]
    fn build_gemini_with_model() {
        let config = AgentLaunchBuilder::new(AgentId::Gemini)
            .model("gemini-2.5-pro")
            .build();

        assert_eq!(config.command, "gemini");
        assert!(config.args.contains(&"--model".to_string()));
        assert!(config.args.contains(&"gemini-2.5-pro".to_string()));
    }

    #[test]
    fn env_override_wins() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .env("TERM", "dumb")
            .build();

        assert_eq!(config.env_vars.get("TERM"), Some(&"dumb".to_string()));
    }

    #[test]
    fn extra_args_appended() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .extra_arg("--verbose")
            .extra_arg("--debug")
            .build();

        assert!(config.args.contains(&"--verbose".to_string()));
        assert!(config.args.contains(&"--debug".to_string()));
    }

    #[test]
    fn build_claude_has_telemetry_disable_vars() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();

        assert_eq!(
            config.env_vars.get("CLAUDE_CODE_NO_FLICKER"),
            Some(&"1".to_string())
        );
        assert_eq!(
            config.env_vars.get("DISABLE_TELEMETRY"),
            Some(&"1".to_string())
        );
        assert_eq!(
            config.env_vars.get("DISABLE_ERROR_REPORTING"),
            Some(&"1".to_string())
        );
        assert_eq!(
            config.env_vars.get("DISABLE_FEEDBACK_COMMAND"),
            Some(&"1".to_string())
        );
        assert_eq!(
            config.env_vars.get("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY"),
            Some(&"1".to_string())
        );
        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"),
            Some(&"1".to_string())
        );
    }

    #[test]
    fn build_claude_auto_permission_mode() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .permission_mode(PermissionMode::Auto)
            .build();

        assert!(config.args.contains(&"--permission-mode".to_string()));
        assert!(config.args.contains(&"auto".to_string()));
    }

    #[test]
    fn resolve_runner_installed_returns_direct_command() {
        let runner = resolve_runner(&AgentId::ClaudeCode, "installed");
        assert_eq!(runner.executable, "claude");
        assert!(runner.base_args.is_empty());
    }

    #[test]
    fn resolve_runner_empty_version_returns_direct_command() {
        let runner = resolve_runner(&AgentId::Codex, "");
        assert_eq!(runner.executable, "codex");
        assert!(runner.base_args.is_empty());
    }

    #[test]
    fn resolve_runner_latest_uses_bunx_or_npx() {
        let runner = resolve_runner(&AgentId::ClaudeCode, "latest");
        assert!(!runner.executable.is_empty());
        let spec_arg = runner.base_args.iter().find(|a| a.contains('@'));
        assert!(spec_arg.is_some(), "should have @package@latest arg");
        assert!(
            spec_arg
                .unwrap()
                .contains("@anthropic-ai/claude-code@latest")
        );
    }

    #[test]
    fn resolve_runner_specific_version_uses_bunx_or_npx() {
        let runner = resolve_runner(&AgentId::Codex, "1.5.0");
        let spec_arg = runner.base_args.iter().find(|a| a.contains('@'));
        assert!(spec_arg.is_some());
        assert!(spec_arg.unwrap().contains("@openai/codex@1.5.0"));
    }

    #[test]
    fn resolve_runner_no_npm_package_falls_back_to_direct() {
        let runner = resolve_runner(&AgentId::OpenCode, "latest");
        assert_eq!(runner.executable, "opencode");
        assert!(runner.base_args.is_empty());
    }

    #[test]
    fn build_with_version_latest() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .version("latest")
            .build();
        assert!(
            config.command.contains("bunx") || config.command.contains("npx"),
            "expected bunx/npx but got: {}",
            config.command
        );
        let has_package_spec = config
            .args
            .iter()
            .any(|a| a.contains("@anthropic-ai/claude-code@latest"));
        assert!(
            has_package_spec,
            "args should contain package@latest: {:?}",
            config.args
        );
    }

    #[test]
    fn build_with_version_installed() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .version("installed")
            .build();
        assert_eq!(config.command, "claude");
    }

    #[test]
    fn custom_agent_minimal() {
        let config = AgentLaunchBuilder::new(AgentId::Custom("aider".into()))
            .extra_arg("--no-git")
            .build();

        assert_eq!(config.command, "aider");
        assert_eq!(config.display_name, "aider");
        assert_eq!(config.color, AgentColor::Gray);
        assert!(config.args.contains(&"--no-git".to_string()));
    }
}
