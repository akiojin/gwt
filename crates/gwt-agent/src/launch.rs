//! Agent launch builder: construct launch configurations for coding agents.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use crate::{
    custom::{CustomAgentType, CustomCodingAgent},
    environment::{host_process_env, hydrate_host_base_env},
    session::GWT_SESSION_RUNTIME_PATH_ENV,
    types::{AgentColor, AgentId, DockerLifecycleIntent, LaunchRuntimeTarget, SessionMode},
};

/// Build the Claude Code `--settings` inline JSON for session-level toggles.
///
/// Both `fastMode` and `ultracode` ride the single `--settings` channel
/// because Claude Code's handling of multiple `--settings` flags is
/// undocumented. Key order is fixed (`fastMode` then `ultracode`) so the
/// emitted string is deterministic and testable. Returns `None` when no toggle
/// is active so the caller emits no flag.
fn claude_session_settings_json(fast_mode: bool, ultracode: bool) -> Option<String> {
    let mut entries: Vec<&str> = Vec::new();
    if fast_mode {
        entries.push(r#""fastMode":true"#);
    }
    if ultracode {
        entries.push(r#""ultracode":true"#);
    }
    if entries.is_empty() {
        return None;
    }
    Some(format!("{{{}}}", entries.join(",")))
}

/// Stable settings filename suffix for the active toggle combination.
fn claude_settings_file_suffix(fast_mode: bool, ultracode: bool) -> &'static str {
    match (fast_mode, ultracode) {
        (true, true) => "fast-ultracode",
        (true, false) => "fast",
        _ => "ultracode",
    }
}

/// Materialize the session settings JSON under `dir` and return the file
/// path to pass as `--settings <path>` (SPEC-2014 FR-106).
///
/// Host launches must not place the JSON on a shell command line: Windows
/// PowerShell wrappers mangle embedded quotes for `.cmd` targets, breaking
/// `--settings {"fastMode":true}` into invalid JSON. The filename is keyed
/// by the toggle combination so concurrent launches with the same toggles
/// write identical bytes and the directory never accumulates stale files.
fn materialize_claude_settings_file(
    dir: &Path,
    fast_mode: bool,
    ultracode: bool,
    json: &str,
) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let path = dir.join(format!(
        "claude-settings-{}.json",
        claude_settings_file_suffix(fast_mode, ultracode)
    ));
    std::fs::write(&path, json)?;
    Ok(path)
}

/// Resolve the gwt repo hash for the directory by shelling out to
/// `git remote get-url origin`. Returns `None` when no origin is configured.
fn detect_repo_hash_for_dir(dir: &Path) -> Option<String> {
    let output = gwt_core::process::hidden_command("git")
        .arg("remote")
        .arg("get-url")
        .arg("origin")
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if url.is_empty() {
        return None;
    }
    Some(
        gwt_core::repo_hash::compute_repo_hash(&url)
            .as_str()
            .to_string(),
    )
}

/// Resolve the gwt worktree hash for the directory.
fn compute_worktree_hash_for_dir(dir: &Path) -> Option<String> {
    let abs = if dir.is_absolute() {
        dir.to_path_buf()
    } else {
        std::env::current_dir().ok()?.join(dir)
    };
    gwt_core::worktree_hash::compute_worktree_hash(&abs)
        .ok()
        .map(|h| h.as_str().to_string())
}

/// Resolved runner command for agent execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRunner {
    /// Executable to invoke (e.g., "claude", "bunx", "npx").
    pub executable: String,
    /// Base args inserted before agent-specific args (e.g., `["@anthropic-ai/claude-code@1.2.3"]`).
    pub base_args: Vec<String>,
}

fn command_basename(command: &str) -> &str {
    Path::new(command)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(command)
}

fn codex_runner_prefix_len(command: &str, args: &[String]) -> Option<usize> {
    match command_basename(command) {
        "codex" => Some(0),
        "bunx" | "npx" => {
            let mut index = 0usize;
            if args.get(index).is_some_and(|arg| arg == "--yes") {
                index += 1;
            }
            args.get(index)
                .is_some_and(|arg| arg.contains("@openai/codex"))
                .then_some(index + 1)
        }
        _ => None,
    }
}

/// Canonical source of truth for agent-neutral default launch arguments.
///
/// Every agent launch entry point — wizard (`AgentLaunchBuilder::build`),
/// preset spawn (`crates/gwt/src/preset.rs`), and persisted session migration
/// (`Session::migrate_legacy_launch_args`) — routes through this function so
/// a default like `--no-alt-screen` cannot silently miss an entry point.
/// See SPEC-1921 FR-064 / Issue #2091 for background.
///
/// This returns only the *agent-neutral* positional defaults. Agent-specific
/// env vars and conditional flags (model, session-mode, fast-mode, reasoning,
/// etc.) remain the responsibility of the agent-specific builder methods.
pub fn canonical_launch_args(agent: &AgentId) -> Vec<String> {
    match agent {
        // Keep Codex out of the alternate screen so the PTY emits normal
        // scrollback instead of redraw-only fullscreen frames. Matches the
        // CLI's documented inline mode for preserving terminal history.
        AgentId::Codex => vec!["--no-alt-screen".to_string()],
        AgentId::ClaudeCode
        | AgentId::Antigravity
        | AgentId::Gemini
        | AgentId::OpenCode
        | AgentId::OpenClaw
        | AgentId::Hermes
        | AgentId::Copilot
        | AgentId::Custom(_) => Vec::new(),
    }
}

pub fn normalize_launch_args(agent_id: &AgentId, command: &str, args: &mut Vec<String>) {
    if !matches!(agent_id, AgentId::Codex) {
        return;
    }
    let Some(insert_index) = codex_runner_prefix_len(command, args) else {
        return;
    };
    for canonical in canonical_launch_args(agent_id).iter().rev() {
        if args.iter().any(|existing| existing == canonical) {
            continue;
        }
        args.insert(insert_index, canonical.clone());
    }
}

/// Resolve the runner command based on version selection.
///
/// - `"installed"` → use the agent's direct command (must be in PATH).
/// - `"latest"` or a semver string → use bunx/npx with `@package@version`.
pub fn resolve_runner(agent_id: &AgentId, version: &str) -> ResolvedRunner {
    let env = host_process_env();
    resolve_runner_with_env(agent_id, version, &env)
}

fn resolve_runner_with_env(
    agent_id: &AgentId,
    version: &str,
    env: &HashMap<String, String>,
) -> ResolvedRunner {
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
        format!("{package}@latest")
    } else {
        format!("{package}@{version}")
    };

    let (executable, needs_yes) = find_bunx_or_npx_for_agent_with_env(agent_id, env);
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

fn resolve_custom_runner(agent: &CustomCodingAgent) -> ResolvedRunner {
    match agent.agent_type {
        CustomAgentType::Command | CustomAgentType::Path => ResolvedRunner {
            executable: agent.command.clone(),
            base_args: Vec::new(),
        },
        CustomAgentType::Bunx => {
            let (executable, needs_yes) = find_bunx_or_npx();
            let mut base_args = Vec::new();
            if needs_yes {
                base_args.push("--yes".to_string());
            }
            base_args.push(agent.command.clone());
            ResolvedRunner {
                executable,
                base_args,
            }
        }
    }
}

/// Platform-specific priority list of package-runner executables to probe via
/// `which::which`. Each entry is `(name, needs_yes)`.
///
/// On Windows, `.cmd` variants come first because `which::which` returns the
/// bare POSIX-shim sibling first (see SPEC-1921 FR-080). `CreateProcess` can
/// execute `.cmd` directly (via `cmd.exe` wrapping); the bare bash shim
/// cannot. On non-Windows platforms the bare names are the canonical entry.
fn package_runner_candidates() -> &'static [(&'static str, bool)] {
    #[cfg(windows)]
    {
        &[
            ("bunx.cmd", false),
            ("bunx", false),
            ("npx.cmd", true),
            ("npx", true),
        ]
    }
    #[cfg(not(windows))]
    {
        &[("bunx", false), ("npx", true)]
    }
}

/// Find bunx or npx executable, preferring global bunx over local node_modules.
///
/// Returns `(executable_name, needs_yes_flag)`.
/// - bunx: no `--yes` needed
/// - npx: `--yes` needed to suppress interactive prompt
fn find_bunx_or_npx() -> (String, bool) {
    let env = host_process_env();
    find_bunx_or_npx_with_env(&env)
}

fn find_bunx_or_npx_with_env(env: &HashMap<String, String>) -> (String, bool) {
    find_package_runner_with_env(package_runner_candidates(), env)
}

fn find_bunx_or_npx_for_agent_with_env(
    agent_id: &AgentId,
    env: &HashMap<String, String>,
) -> (String, bool) {
    find_package_runner_with_env(package_runner_candidates_for_agent(agent_id), env)
}

fn package_runner_candidates_for_agent(agent_id: &AgentId) -> &'static [(&'static str, bool)] {
    #[cfg(windows)]
    {
        let _ = agent_id;
        package_runner_candidates()
    }
    #[cfg(not(windows))]
    {
        if matches!(agent_id, AgentId::ClaudeCode) {
            // Bun can leave Claude Code's postinstall-managed native binary as a stub
            // in one-shot package runs; npx executes the published package reliably.
            &[("npx", true), ("bunx", false)]
        } else {
            package_runner_candidates()
        }
    }
}

fn find_package_runner_with_env(
    candidates: &'static [(&'static str, bool)],
    env: &HashMap<String, String>,
) -> (String, bool) {
    let env = hydrate_host_base_env(env.clone());
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let path = env.get("PATH").map(String::as_str);
    find_package_runner_in_path(candidates, path, &cwd).unwrap_or_else(|| {
        // Last resort: assume bunx is available
        ("bunx".to_string(), false)
    })
}

fn find_package_runner_in_path(
    candidates: &'static [(&'static str, bool)],
    path: Option<&str>,
    cwd: &Path,
) -> Option<(String, bool)> {
    for (name, needs_yes) in candidates {
        let found = match path {
            Some(path) => which::which_in(name, Some(path), cwd),
            None => which::which(name),
        };
        if let Ok(found) = found {
            let path_str = found.to_string_lossy();
            if path_str.contains("node_modules") {
                continue;
            }
            return Some((path_str.into_owned(), *needs_yes));
        }
    }
    None
}

/// Platform priority list of `npx` fallback executables consulted when the host
/// `bunx` package-runner probe fails (Issue #2981). On Windows the `.cmd`
/// variant comes first so `CreateProcess` can spawn it; the bare `npx` POSIX
/// shim is not directly spawnable there (SPEC-1921 FR-080). Other platforms use
/// the bare canonical name.
fn npx_fallback_candidates() -> &'static [(&'static str, bool)] {
    #[cfg(windows)]
    {
        &[("npx.cmd", true), ("npx", true)]
    }
    #[cfg(not(windows))]
    {
        &[("npx", true)]
    }
}

/// Resolve the host `npx` fallback executable used when the `bunx`
/// package-runner probe fails during launch.
///
/// Resolution flows through the same Windows-aware `find_package_runner_in_path`
/// machinery as the primary runner, so on Windows the spawnable `npx.cmd` is
/// preferred over the bare `npx` shim (Issue #2981). Falls back to the canonical
/// bare `"npx"` name when no candidate resolves on the launch `PATH`, preserving
/// the prior behavior for environments without a resolvable npx.
pub fn resolve_host_npx_fallback_executable(env: &HashMap<String, String>) -> String {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    find_package_runner_in_path(
        npx_fallback_candidates(),
        env.get("PATH").map(String::as_str),
        &cwd,
    )
    .map(|(executable, _needs_yes)| executable)
    .unwrap_or_else(|| "npx".to_string())
}

/// Final configuration used to spawn an agent process.
#[derive(Debug, Clone)]
pub struct LaunchConfig {
    pub agent_id: AgentId,
    pub command: String,
    pub args: Vec<String>,
    pub env_vars: HashMap<String, String>,
    pub remove_env: Vec<String>,
    pub working_dir: Option<PathBuf>,
    pub branch: Option<String>,
    pub base_branch: Option<String>,
    pub display_name: String,
    pub color: AgentColor,
    pub model: Option<String>,
    pub tool_version: Option<String>,
    pub reasoning_level: Option<String>,
    pub session_mode: SessionMode,
    pub resume_session_id: Option<String>,
    pub skip_permissions: bool,
    pub fast_mode: bool,
    /// Legacy Codex-only compatibility field. New callers should use
    /// `fast_mode`; this remains populated for persisted/session consumers
    /// that still distinguish Codex's service-tier implementation.
    pub codex_fast_mode: bool,
    pub runtime_target: LaunchRuntimeTarget,
    pub docker_service: Option<String>,
    pub docker_lifecycle_intent: DockerLifecycleIntent,
    pub linked_issue_number: Option<u64>,
    pub windows_shell: Option<crate::WindowsShellKind>,
    /// SPEC-3214: this launch runs in an ephemeral, detached intake worktree
    /// that is removed when the session ends. `true` routes worktree
    /// resolution through the detached-worktree path instead of creating a
    /// branch.
    pub is_ephemeral: bool,
    /// Base committish for the ephemeral intake worktree (e.g. `origin/develop`).
    /// `None` defaults to `HEAD`. Only meaningful when `is_ephemeral` is set.
    pub ephemeral_base_ref: Option<String>,
}

/// Permission mode for agent launch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionMode {
    Default,
    AcceptEdits,
    Plan,
    DontAsk,
    BypassPermissions,
}

/// Builder for constructing agent launch configurations.
#[derive(Debug, Clone)]
pub struct AgentLaunchBuilder {
    agent_id: AgentId,
    custom_agent: Option<CustomCodingAgent>,
    working_dir: Option<PathBuf>,
    branch: Option<String>,
    base_branch: Option<String>,
    model: Option<String>,
    version: Option<String>,
    fast_mode: bool,
    skip_permissions: bool,
    reasoning_level: Option<String>,
    session_mode: SessionMode,
    resume_session_id: Option<String>,
    permission_mode: Option<PermissionMode>,
    env_overrides: HashMap<String, String>,
    /// Env table from a `CustomCodingAgent`. Merged into the spawn env AFTER
    /// the SPEC-1921 Common family (TERM / GWT_*) and agent-specific env
    /// (Claude / Codex / …), and BEFORE the explicit `env_overrides` field
    /// above, so preset-seeded custom entries win over built-in defaults
    /// but never clobber explicit caller-provided overrides.
    custom_agent_env: HashMap<String, String>,
    /// Active Backend Override profile (SPEC-1921 FR-100 / FR-103).
    /// `None` means the agent launches against its default upstream
    /// (no env override). Set only for built-in agents that support
    /// Backend Override (Claude Code, Codex).
    backend_profile: Option<crate::backend::AgentBackendProfile>,
    /// Hermes-specific launch options (SPEC-3152). `provider` maps to
    /// `--provider`, `profile` to `--profile`, `toolsets`/`skills` to the
    /// CSV/`--skills` flags, `max_turns` to `--max-turns`, and `safe_mode`
    /// to `--safe-mode`. Ignored by agents that do not consume them.
    provider: Option<String>,
    profile: Option<String>,
    toolsets: Option<String>,
    skills: Option<String>,
    max_turns: Option<u32>,
    safe_mode: bool,
    extra_args: Vec<String>,
    runtime_target: LaunchRuntimeTarget,
    docker_service: Option<String>,
    docker_lifecycle_intent: DockerLifecycleIntent,
    linked_issue_number: Option<u64>,
    windows_shell: Option<crate::WindowsShellKind>,
    is_ephemeral: bool,
    ephemeral_base_ref: Option<String>,
}

impl AgentLaunchBuilder {
    pub fn new(agent_id: AgentId) -> Self {
        Self {
            agent_id,
            custom_agent: None,
            working_dir: None,
            branch: None,
            base_branch: None,
            model: None,
            version: None,
            fast_mode: false,
            skip_permissions: false,
            reasoning_level: None,
            session_mode: SessionMode::Normal,
            resume_session_id: None,
            permission_mode: None,
            env_overrides: HashMap::new(),
            custom_agent_env: HashMap::new(),
            backend_profile: None,
            provider: None,
            profile: None,
            toolsets: None,
            skills: None,
            max_turns: None,
            safe_mode: false,
            extra_args: Vec::new(),
            runtime_target: LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: DockerLifecycleIntent::Connect,
            linked_issue_number: None,
            windows_shell: None,
            is_ephemeral: false,
            ephemeral_base_ref: None,
        }
    }

    /// SPEC-3214: mark this launch as an ephemeral intake session that runs in
    /// a detached, throwaway worktree based on `base_ref` (`None` → `HEAD`).
    pub fn ephemeral(mut self, base_ref: Option<String>) -> Self {
        self.is_ephemeral = true;
        self.ephemeral_base_ref = base_ref;
        self
    }

    pub fn custom_agent(mut self, agent: CustomCodingAgent) -> Self {
        self.custom_agent = Some(agent);
        self
    }

    pub fn working_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        self.working_dir = Some(gwt_core::paths::normalize_windows_child_process_path(&dir));
        self
    }

    pub fn branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = Some(branch.into());
        self
    }

    pub fn base_branch(mut self, branch: impl Into<String>) -> Self {
        self.base_branch = Some(branch.into());
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

    pub fn skip_permissions(mut self, enabled: bool) -> Self {
        self.skip_permissions = enabled;
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

    /// Supply a `CustomCodingAgent.env` table that should flow into the
    /// spawn env. Intended for SPEC-1921 FR-063: preset-seeded env tables
    /// merge AFTER Common and agent-specific env, BEFORE the explicit
    /// `env_overrides` field, so preset entries win over built-in
    /// defaults but never clobber explicit caller overrides.
    pub fn custom_agent_env(mut self, env: HashMap<String, String>) -> Self {
        self.custom_agent_env = env;
        self
    }

    /// SPEC-1921 FR-100 / FR-103: attach an Agent Backend Override profile.
    /// Claude Code launches inject `ANTHROPIC_BASE_URL`, `ANTHROPIC_API_KEY`,
    /// and the four model-role env vars; Codex launches consume the profile
    /// through the worktree-local `CODEX_HOME` generator (FR-103 / Phase
    /// 63E). The default upstream is used when no profile is set.
    pub fn backend_profile(mut self, profile: crate::backend::AgentBackendProfile) -> Self {
        self.backend_profile = Some(profile);
        self
    }

    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = Some(mode);
        self
    }

    /// SPEC-3152: Hermes provider selection (`--provider`), e.g. `openrouter`,
    /// `nous`, `anthropic`. Ignored by agents that do not consume it.
    pub fn provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = Some(provider.into());
        self
    }

    /// SPEC-3152: Hermes profile selection (`--profile`).
    pub fn profile(mut self, profile: impl Into<String>) -> Self {
        self.profile = Some(profile.into());
        self
    }

    /// SPEC-3152: Hermes toolsets CSV (`--toolsets`).
    pub fn toolsets(mut self, toolsets: impl Into<String>) -> Self {
        self.toolsets = Some(toolsets.into());
        self
    }

    /// SPEC-3152: Hermes preloaded skills (`--skills`).
    pub fn skills(mut self, skills: impl Into<String>) -> Self {
        self.skills = Some(skills.into());
        self
    }

    /// SPEC-3152: Hermes per-turn tool-call cap (`--max-turns`).
    pub fn max_turns(mut self, turns: u32) -> Self {
        self.max_turns = Some(turns);
        self
    }

    /// SPEC-3152: Hermes safe-mode (`--safe-mode`). Note: safe-mode disables
    /// user config / rules / plugins, which also disables gwt hooks.
    pub fn safe_mode(mut self, enabled: bool) -> Self {
        self.safe_mode = enabled;
        self
    }

    pub fn extra_arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    pub fn runtime_target(mut self, target: LaunchRuntimeTarget) -> Self {
        self.runtime_target = target;
        self
    }

    pub fn docker_service(mut self, service: impl Into<String>) -> Self {
        self.docker_service = Some(service.into());
        self
    }

    pub fn docker_lifecycle_intent(mut self, intent: DockerLifecycleIntent) -> Self {
        self.docker_lifecycle_intent = intent;
        self
    }

    pub fn linked_issue_number(mut self, n: u64) -> Self {
        self.linked_issue_number = Some(n);
        self
    }

    pub fn windows_shell(mut self, shell: crate::WindowsShellKind) -> Self {
        self.windows_shell = Some(shell);
        self
    }

    /// Build the final `LaunchConfig`.
    pub fn build(mut self) -> LaunchConfig {
        self.working_dir = self
            .working_dir
            .as_ref()
            .map(|dir| gwt_core::paths::normalize_windows_child_process_path(dir));
        let mut env_vars = HashMap::new();
        let skip_permissions = self.skip_permissions
            || matches!(
                self.permission_mode,
                Some(PermissionMode::BypassPermissions)
            );

        // Common env vars
        env_vars.insert("TERM".to_string(), "xterm-256color".to_string());
        if let Some(ref dir) = self.working_dir {
            env_vars.insert("GWT_PROJECT_ROOT".to_string(), dir.display().to_string());

            // Phase 8 / SPEC-10 FR-028: export repo & worktree hashes so the
            // skill-driven runner calls can reconstruct the DB path without
            // re-deriving via `git remote` on every invocation.
            if let Some(repo_hash) = detect_repo_hash_for_dir(dir) {
                env_vars.insert("GWT_REPO_HASH".to_string(), repo_hash);
            }
            if let Some(wt_hash) = compute_worktree_hash_for_dir(dir) {
                env_vars.insert("GWT_WORKTREE_HASH".to_string(), wt_hash);
            }
        }

        // Resolve runner (installed binary vs bunx/npx)
        let runner = self
            .custom_agent
            .as_ref()
            .map(resolve_custom_runner)
            .unwrap_or_else(|| {
                resolve_runner(
                    &self.agent_id,
                    self.version.as_deref().unwrap_or("installed"),
                )
            });

        let mut args = runner.base_args;
        if let Some(custom_agent) = self.custom_agent.as_ref() {
            args.extend(custom_agent.build_args(self.session_mode));
            if skip_permissions {
                args.extend(custom_agent.skip_permissions_args.clone());
            }
        }

        // Agent-specific configuration
        match &self.agent_id {
            AgentId::ClaudeCode => {
                self.build_claude_args(&mut args, &mut env_vars);
            }
            AgentId::Codex => {
                self.build_codex_args(&mut args, &mut env_vars);
            }
            AgentId::Antigravity => {
                self.build_antigravity_args(&mut args);
            }
            AgentId::Gemini => {
                self.build_gemini_args(&mut args);
            }
            AgentId::OpenCode => {
                self.build_opencode_args(&mut args, &mut env_vars);
            }
            AgentId::OpenClaw => {
                self.build_openclaw_args(&mut args, &mut env_vars);
            }
            AgentId::Hermes => {
                self.build_hermes_args(&mut args, &mut env_vars);
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

        normalize_launch_args(&self.agent_id, &runner.executable, &mut args);

        // SPEC-1921 FR-063: merge CustomCodingAgent.env AFTER Common and
        // agent-specific env so preset entries win over built-in defaults
        // but BEFORE env_overrides (explicit caller overrides still win).
        if let Some(custom_agent) = self.custom_agent.as_ref() {
            env_vars.extend(custom_agent.env.clone());
        }
        env_vars.extend(self.custom_agent_env);

        // Apply env overrides last (user wins)
        env_vars.extend(self.env_overrides);

        let agent_id = self.agent_id.clone();
        let display_name = self
            .custom_agent
            .as_ref()
            .map(|agent| agent.display_name.clone())
            .unwrap_or_else(|| self.agent_id.display_name().to_string());
        let color = self.agent_id.default_color();
        let model = self.model.clone();
        let tool_version = self
            .version
            .clone()
            .filter(|version| version != "installed");
        let reasoning_level = self.reasoning_level.clone();
        let session_mode = self.session_mode;
        let resume_session_id = self.resume_session_id.clone();
        let fast_mode = self.fast_mode && self.agent_id.supports_fast_mode();
        let codex_fast_mode = matches!(self.agent_id, AgentId::Codex) && self.fast_mode;

        LaunchConfig {
            agent_id,
            command: runner.executable,
            args,
            env_vars,
            remove_env: Vec::new(),
            working_dir: self.working_dir,
            branch: self.branch,
            base_branch: self.base_branch,
            display_name,
            color,
            model,
            tool_version,
            reasoning_level,
            session_mode,
            resume_session_id,
            skip_permissions,
            fast_mode,
            codex_fast_mode,
            runtime_target: self.runtime_target,
            docker_service: self.docker_service,
            docker_lifecycle_intent: self.docker_lifecycle_intent,
            linked_issue_number: self.linked_issue_number,
            windows_shell: self.windows_shell,
            is_ephemeral: self.is_ephemeral,
            ephemeral_base_ref: self.ephemeral_base_ref,
        }
    }

    fn build_claude_args(&self, args: &mut Vec<String>, env_vars: &mut HashMap<String, String>) {
        // Claude Code specific env vars
        env_vars.insert("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".into(), "1".into());

        // Telemetry/analytics disable
        env_vars.insert("DISABLE_TELEMETRY".into(), "1".into());
        env_vars.insert("DISABLE_ERROR_REPORTING".into(), "1".into());
        env_vars.insert("DISABLE_FEEDBACK_COMMAND".into(), "1".into());
        env_vars.insert("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY".into(), "1".into());
        // Claude Code's fullscreen/no-flicker renderer keeps virtualized
        // history inside Claude instead of xterm, so gwt agent windows have no
        // normal scrollback to wheel/trackpad through.
        env_vars.insert("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN".into(), "1".into());
        env_vars.insert("CLAUDE_CODE_NO_FLICKER".into(), "0".into());

        // SPEC-1921 FR-100 (2026-05-18 amendment): Backend Override env
        // injection. When the launch carries a `[builtinAgents.claudeCode.
        // backends.<id>]` profile, route Claude Code's Anthropic Messages
        // API traffic to the upstream proxy by setting `ANTHROPIC_BASE_URL`
        // plus the four model-role overrides and the OpenAI-compat-friendly
        // telemetry-off flags. The default path (no profile) leaves the
        // upstream untouched so the agent talks to Anthropic directly.
        if let Some(profile) = self.backend_profile.as_ref() {
            env_vars.insert("ANTHROPIC_BASE_URL".into(), profile.base_url.clone());
            env_vars.insert("ANTHROPIC_API_KEY".into(), profile.api_key.clone());
            env_vars.insert(
                "ANTHROPIC_DEFAULT_HAIKU_MODEL".into(),
                profile.effective_haiku_model().to_string(),
            );
            env_vars.insert(
                "ANTHROPIC_DEFAULT_OPUS_MODEL".into(),
                profile.effective_opus_model().to_string(),
            );
            env_vars.insert(
                "ANTHROPIC_DEFAULT_SONNET_MODEL".into(),
                profile.effective_sonnet_model().to_string(),
            );
            env_vars.insert(
                "CLAUDE_CODE_SUBAGENT_MODEL".into(),
                profile.effective_subagent_model().to_string(),
            );
            // Suppress non-essential traffic so a non-Anthropic upstream
            // does not see unrelated telemetry POSTs (SPEC-1921 Phase 52
            // preset legacy invariant).
            env_vars.insert("CLAUDE_CODE_ATTRIBUTION_HEADER".into(), "0".into());
            env_vars.insert(
                "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC".into(),
                "1".into(),
            );
        }

        // Permission mode
        if let Some(ref mode) = self.permission_mode {
            args.push("--permission-mode".to_string());
            args.push(
                match mode {
                    PermissionMode::Default => "default",
                    PermissionMode::AcceptEdits => "acceptEdits",
                    PermissionMode::Plan => "plan",
                    PermissionMode::DontAsk => "dontAsk",
                    PermissionMode::BypassPermissions => "bypassPermissions",
                }
                .to_string(),
            );
        }

        if self.skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }

        // Session-level settings overrides (fast mode and/or ultracode) are
        // merged into a single `--settings` flag. `ultracode` is selected via
        // the reasoning level but activated here as a Claude Code session
        // setting (it implies xhigh + workflow orchestration), never as an
        // effort-level value.
        let ultracode = self.reasoning_level.as_deref() == Some("ultracode");
        if let Some(settings) = claude_session_settings_json(self.fast_mode, ultracode) {
            // SPEC-2014 FR-106: host launches deliver the settings as a file
            // path so the JSON never rides a shell command line. Docker keeps
            // the inline form: its args travel as an argv vector with no
            // quoting layer, and the host-side file is not guaranteed to be
            // mounted in the container.
            let value = if self.runtime_target == LaunchRuntimeTarget::Host {
                match materialize_claude_settings_file(
                    &gwt_core::paths::gwt_home().join("tmp"),
                    self.fast_mode,
                    ultracode,
                    &settings,
                ) {
                    Ok(path) => path.display().to_string(),
                    Err(err) => {
                        tracing::warn!(
                            error = %err,
                            "failed to materialize claude session settings file; \
                             falling back to inline JSON"
                        );
                        settings
                    }
                }
            } else {
                settings
            };
            args.push("--settings".to_string());
            args.push(value);
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

        if let Some(ref level) = self.reasoning_level {
            // "auto" lets the model choose; "ultracode" is delivered via
            // --settings above (and implies xhigh), not a valid
            // CLAUDE_CODE_EFFORT_LEVEL value.
            if level != "auto" && level != "ultracode" {
                env_vars.insert("CLAUDE_CODE_EFFORT_LEVEL".to_string(), level.clone());
            }
        }
    }

    fn build_codex_args(&self, args: &mut Vec<String>, env_vars: &mut HashMap<String, String>) {
        env_vars.insert(
            "CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS".to_string(),
            "1".to_string(),
        );

        // SPEC-1921 FR-103 (2026-05-18 amendment): when a Codex Backend
        // Override profile is attached, materialize a worktree-local
        // CODEX_HOME containing a generated
        // `[model_providers.<gwt-id>]` config.toml and point Codex at
        // it through the CODEX_HOME env var plus the API key env var
        // matching the profile's `env_key`. The default path (no profile)
        // leaves Codex pointing at the user's `~/.codex/`.
        if let Some(profile) = self.backend_profile.as_ref() {
            let cfg = gwt_skills::CodexHomeConfig {
                id: profile.id.clone(),
                display_name: profile.display_name.clone(),
                base_url: profile.base_url.clone(),
                model: profile.model.clone(),
                wire_api: profile.wire_api.clone(),
                env_key: profile.env_key.clone(),
                provider_id: profile.provider_id.clone(),
            };
            let codex_home = self
                .working_dir
                .as_ref()
                .map(|root| gwt_skills::codex_home_for_worktree(root))
                .unwrap_or_else(|| std::env::temp_dir().join(".gwt-codex"));
            if let Err(err) = gwt_skills::materialize_codex_home(&codex_home, &cfg) {
                tracing::warn!(
                    error = %err,
                    backend_id = %profile.id,
                    "failed to materialize CODEX_HOME; Codex backend env still set"
                );
            }
            env_vars.insert("CODEX_HOME".to_string(), codex_home.display().to_string());
            env_vars.insert(cfg.effective_env_key(), profile.api_key.clone());
        }

        args.extend(canonical_launch_args(&AgentId::Codex));
        // SPEC-2014 2026-05-18 amendment FR-B:
        // - Continue        → `codex resume --last`  (resume the most recent session)
        // - Resume + id     → `codex resume <id>`    (Quick Start: replay specific session)
        // - Resume (no id)  → `codex resume`         (Execution Mode picker; do NOT add `--last`)
        match self.session_mode {
            SessionMode::Continue => {
                args.push("resume".to_string());
                args.push("--last".to_string());
            }
            SessionMode::Resume => {
                args.push("resume".to_string());
                if let Some(ref id) = self.resume_session_id {
                    args.push(id.clone());
                }
            }
            SessionMode::Normal => {}
        }

        if let Some(ref model) = self.model {
            args.push(format!("--model={model}"));
        }

        // Reasoning level (Codex-specific). `ultracode` is a Claude-only
        // session setting and not a valid Codex reasoning effort, so skip the
        // override defensively (Codex falls back to its own default) instead of
        // emitting `model_reasoning_effort=ultracode`, which would fail to start.
        if let Some(ref level) = self.reasoning_level {
            if level != "ultracode" {
                args.push("-c".to_string());
                args.push(format!("model_reasoning_effort={level}"));
                args.push("-c".to_string());
                args.push("model_reasoning_summaries=detailed".to_string());
            }
        }

        // Version-dependent flags
        let version_str = self.version.as_deref().unwrap_or("");
        let parsed_version = semver::Version::parse(version_str).ok();

        if self.fast_mode
            && parsed_version
                .as_ref()
                .is_some_and(|ver| *ver >= semver::Version::new(0, 110, 0))
        {
            args.push("-c".to_string());
            args.push("service_tier=fast".to_string());
        }

        if self.skip_permissions {
            args.push("--yolo".to_string());
        }

        args.push("--enable".to_string());
        args.push("goals".to_string());

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

        if let Some(runtime_dir) = self.codex_runtime_writable_root(env_vars) {
            args.push("--add-dir".to_string());
            args.push(runtime_dir);
        }
    }

    fn codex_runtime_writable_root(&self, env_vars: &HashMap<String, String>) -> Option<String> {
        self.env_overrides
            .get(GWT_SESSION_RUNTIME_PATH_ENV)
            .or_else(|| env_vars.get(GWT_SESSION_RUNTIME_PATH_ENV))
            .map(PathBuf::from)
            .and_then(|runtime_path| runtime_path.parent().map(std::path::Path::to_path_buf))
            .map(|dir| dir.to_string_lossy().into_owned())
    }

    fn build_gemini_args(&self, args: &mut Vec<String>) {
        if let Some(ref model) = self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        if self.skip_permissions {
            args.push("--yolo".to_string());
        }
    }

    fn build_antigravity_args(&self, args: &mut Vec<String>) {
        match self.session_mode {
            SessionMode::Continue => args.push("--continue".to_string()),
            SessionMode::Resume => {
                if let Some(ref id) = self.resume_session_id {
                    args.push("--conversation".to_string());
                    args.push(id.clone());
                }
            }
            SessionMode::Normal => {}
        }

        if let Some(ref model) = self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }

        if self.skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }
    }

    fn build_opencode_args(&self, args: &mut Vec<String>, env_vars: &mut HashMap<String, String>) {
        if let Some(ref dir) = self.working_dir {
            env_vars.insert(
                "OPENCODE_CONFIG_DIR".to_string(),
                dir.join(".gwt/opencode").to_string_lossy().into_owned(),
            );
            // SPEC-3151 FR-005: OpenCode has no skip-permissions CLI flag; honor
            // the per-launch toggle by layering the permissive permission overlay
            // (generated under .gwt/opencode) on top of OPENCODE_CONFIG_DIR via
            // OPENCODE_CONFIG. Left unset otherwise so OpenCode's default
            // permission prompts apply.
            if self.skip_permissions {
                env_vars.insert(
                    "OPENCODE_CONFIG".to_string(),
                    dir.join(".gwt/opencode/skip-permissions.json")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
        match self.session_mode {
            SessionMode::Continue => args.push("--continue".to_string()),
            SessionMode::Resume => {
                if let Some(ref id) = self.resume_session_id {
                    args.push("--session".to_string());
                    args.push(id.clone());
                } else {
                    args.push("--continue".to_string());
                }
            }
            SessionMode::Normal => {}
        }
        if let Some(ref model) = self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
    }

    fn build_openclaw_args(&self, args: &mut Vec<String>, env_vars: &mut HashMap<String, String>) {
        args.push("tui".to_string());
        args.push("--local".to_string());
        if let Some(ref dir) = self.working_dir {
            env_vars.insert(
                "OPENCLAW_CONFIG_PATH".to_string(),
                dir.join(".gwt/openclaw/openclaw.json")
                    .to_string_lossy()
                    .into_owned(),
            );
            env_vars.insert(
                "OPENCLAW_INCLUDE_ROOTS".to_string(),
                dir.join(".gwt/openclaw").to_string_lossy().into_owned(),
            );
        }
        match self.session_mode {
            SessionMode::Continue => {}
            SessionMode::Resume => {
                if let Some(ref id) = self.resume_session_id {
                    args.push("--session".to_string());
                    args.push(id.clone());
                }
            }
            SessionMode::Normal => {}
        }
        if let Some(ref level) = self.reasoning_level {
            if level != "auto" {
                args.push("--thinking".to_string());
                args.push(level.clone());
            }
        }
    }

    fn build_hermes_args(&self, args: &mut Vec<String>, env_vars: &mut HashMap<String, String>) {
        args.push("chat".to_string());
        args.push("--accept-hooks".to_string());
        args.push("--pass-session-id".to_string());
        if let Some(ref dir) = self.working_dir {
            env_vars.insert(
                "HERMES_HOME".to_string(),
                dir.join(".gwt/hermes").to_string_lossy().into_owned(),
            );
        }
        env_vars.insert("HERMES_ACCEPT_HOOKS".to_string(), "1".to_string());
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
        if let Some(ref provider) = self.provider {
            args.push("--provider".to_string());
            args.push(provider.clone());
        }
        if let Some(ref model) = self.model {
            args.push("--model".to_string());
            args.push(model.clone());
        }
        if let Some(ref profile) = self.profile {
            args.push("--profile".to_string());
            args.push(profile.clone());
        }
        if let Some(ref toolsets) = self.toolsets {
            args.push("--toolsets".to_string());
            args.push(toolsets.clone());
        }
        if let Some(ref skills) = self.skills {
            args.push("--skills".to_string());
            args.push(skills.clone());
        }
        if let Some(max_turns) = self.max_turns {
            args.push("--max-turns".to_string());
            args.push(max_turns.to_string());
        }
        if self.safe_mode {
            args.push("--safe-mode".to_string());
        }
        if self.skip_permissions {
            args.push("--yolo".to_string());
        }
    }

    fn build_copilot_args(&self, args: &mut Vec<String>) {
        // gh copilot is invoked as `gh copilot`
        args.insert(0, "copilot".to_string());
        if self.skip_permissions {
            args.push("--yolo".to_string());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::custom::{CustomAgentType, CustomCodingAgent, ModeArgs};

    use super::*;

    // SPEC-1921 Phase 53 / Issue #2091: canonical_launch_args is the single
    // source of truth for agent-neutral default args across all launch entry
    // points (wizard, preset, session-load migration). Regression guard for
    // the preset-path gap that caused Codex Plan-mode scroll to die.

    fn project_relative_path(relative: &str) -> String {
        std::path::PathBuf::from("/tmp/project")
            .join(relative)
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn canonical_launch_args_for_codex_contains_no_alt_screen() {
        let args = canonical_launch_args(&AgentId::Codex);
        assert!(
            args.iter().any(|arg| arg == "--no-alt-screen"),
            "Codex canonical args must include --no-alt-screen (FR-064, Issue #2091)"
        );
    }

    #[test]
    fn canonical_launch_args_for_non_codex_agents_is_empty() {
        // Claude/Gemini/OpenCode/Copilot/Custom have no agent-neutral positional
        // defaults today. Agent-specific env vars and conditional args belong in
        // the agent-specific builder, not the canonical default list.
        assert!(canonical_launch_args(&AgentId::ClaudeCode).is_empty());
        assert!(canonical_launch_args(&AgentId::Antigravity).is_empty());
        assert!(canonical_launch_args(&AgentId::Gemini).is_empty());
        assert!(canonical_launch_args(&AgentId::OpenCode).is_empty());
        assert!(canonical_launch_args(&AgentId::OpenClaw).is_empty());
        assert!(canonical_launch_args(&AgentId::Hermes).is_empty());
        assert!(canonical_launch_args(&AgentId::Copilot).is_empty());
        assert!(canonical_launch_args(&AgentId::Custom("aider".into())).is_empty());
    }

    #[test]
    fn canonical_launch_args_is_deterministic() {
        // FR-064: same AgentId always yields the same Vec<String>.
        let first = canonical_launch_args(&AgentId::Codex);
        let second = canonical_launch_args(&AgentId::Codex);
        assert_eq!(first, second);
    }

    #[test]
    fn builder_default_state() {
        let builder = AgentLaunchBuilder::new(AgentId::ClaudeCode);
        assert_eq!(builder.agent_id, AgentId::ClaudeCode);
        assert!(builder.working_dir.is_none());
        assert!(builder.base_branch.is_none());
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
        assert_eq!(config.color, AgentColor::Yellow);
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
    fn build_claude_uses_classic_main_screen_renderer_for_scrollback() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();

        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_DISABLE_ALTERNATE_SCREEN")
                .map(String::as_str),
            Some("1"),
            "Claude Code must use the classic main-screen renderer so xterm owns normal scrollback"
        );
        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_NO_FLICKER")
                .map(String::as_str),
            Some("0"),
            "gwt must explicitly disable Claude Code's fullscreen/no-flicker renderer"
        );
    }

    #[test]
    fn windows_launch_paths_builder_normalizes_working_dir_and_project_root() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .working_dir(r"Microsoft.PowerShell.Core\FileSystem::\\?\E:\gwt\work\20260525-0919")
            .build();

        assert_eq!(
            config.working_dir,
            Some(PathBuf::from(r"E:\gwt\work\20260525-0919"))
        );
        assert_eq!(
            config.env_vars.get("GWT_PROJECT_ROOT").map(String::as_str),
            Some(r"E:\gwt\work\20260525-0919")
        );
    }

    #[test]
    fn build_carries_base_branch() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .branch("feature/demo")
            .base_branch("develop")
            .build();

        assert_eq!(config.branch.as_deref(), Some("feature/demo"));
        assert_eq!(config.base_branch.as_deref(), Some("develop"));
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
            .model("claude-opus-4-8")
            .build();

        assert!(config.args.contains(&"--model".to_string()));
        assert!(config.args.contains(&"claude-opus-4-8".to_string()));
    }

    #[test]
    fn build_claude_with_auto_reasoning_does_not_export_effort_env() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .reasoning_level("auto")
            .build();

        assert_eq!(config.reasoning_level.as_deref(), Some("auto"));
        assert!(!config.env_vars.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
        assert!(!config.args.contains(&"--effort".to_string()));
    }

    #[test]
    fn build_claude_with_reasoning_level_exports_effort_env() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .reasoning_level("high")
            .build();

        assert_eq!(config.reasoning_level.as_deref(), Some("high"));
        assert_eq!(
            config.env_vars.get("CLAUDE_CODE_EFFORT_LEVEL"),
            Some(&"high".to_string())
        );
        assert!(!config.args.contains(&"--effort".to_string()));
    }

    #[test]
    fn build_claude_with_xhigh_reasoning_exports_effort_env() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .reasoning_level("xhigh")
            .build();

        assert_eq!(config.reasoning_level.as_deref(), Some("xhigh"));
        assert_eq!(
            config.env_vars.get("CLAUDE_CODE_EFFORT_LEVEL"),
            Some(&"xhigh".to_string())
        );
        assert!(!config.args.contains(&"--effort".to_string()));
    }

    #[test]
    fn build_claude_with_max_reasoning_exports_effort_env() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .reasoning_level("max")
            .build();

        assert_eq!(config.reasoning_level.as_deref(), Some("max"));
        assert_eq!(
            config.env_vars.get("CLAUDE_CODE_EFFORT_LEVEL"),
            Some(&"max".to_string())
        );
        assert!(!config.args.contains(&"--effort".to_string()));
    }

    #[test]
    fn build_claude_skip_permissions_adds_dangerous_flag() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .skip_permissions(true)
            .build();

        assert!(config
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
        assert!(config.skip_permissions);
    }

    /// SPEC-2014 FR-106 / SC-064: host launches carry --settings as a
    /// materialized file path so the JSON never rides a shell command line.
    fn claude_settings_arg(config: &LaunchConfig) -> String {
        config
            .args
            .windows(2)
            .find(|pair| pair[0] == "--settings")
            .map(|pair| pair[1].clone())
            .expect("--settings argument present")
    }

    #[test]
    fn build_claude_fast_mode_host_materializes_settings_file() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .fast_mode(true)
            .build();

        let value = claude_settings_arg(&config);
        assert!(
            value.ends_with("claude-settings-fast.json"),
            "host launches must pass a settings file path, got {value}"
        );
        let content = std::fs::read_to_string(&value).expect("settings file written");
        assert_eq!(content, r#"{"fastMode":true}"#);
        assert!(config.fast_mode);
        assert!(!config.codex_fast_mode);
    }

    #[test]
    fn build_claude_fast_mode_docker_keeps_inline_settings() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .fast_mode(true)
            .runtime_target(LaunchRuntimeTarget::Docker)
            .build();

        // Docker args travel as an argv vector with no quoting layer, and a
        // host-side settings file is not guaranteed to be mounted in the
        // container, so the inline JSON form stays.
        assert_eq!(claude_settings_arg(&config), r#"{"fastMode":true}"#);
    }

    #[test]
    fn build_claude_without_fast_mode_omits_session_settings() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();

        assert!(!config.args.iter().any(|arg| arg == "--settings"));
    }

    #[test]
    fn build_claude_with_ultracode_sets_settings_and_skips_effort_env() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .reasoning_level("ultracode")
            .build();

        // ultracode rides --settings, never CLAUDE_CODE_EFFORT_LEVEL.
        let value = claude_settings_arg(&config);
        assert!(
            value.ends_with("claude-settings-ultracode.json"),
            "host launches must pass a settings file path, got {value}"
        );
        assert_eq!(
            std::fs::read_to_string(&value).expect("settings file written"),
            r#"{"ultracode":true}"#
        );
        assert!(!config.env_vars.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
        assert!(!config.args.contains(&"--effort".to_string()));
        // reasoning_level still round-trips for persistence/display.
        assert_eq!(config.reasoning_level.as_deref(), Some("ultracode"));
    }

    #[test]
    fn build_claude_fast_mode_and_ultracode_combine_settings() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .fast_mode(true)
            .reasoning_level("ultracode")
            .build();

        // Exactly one --settings flag carrying a deterministic combined object.
        let settings_count = config
            .args
            .iter()
            .filter(|arg| *arg == "--settings")
            .count();
        assert_eq!(
            settings_count, 1,
            "must combine into a single --settings flag"
        );
        let value = claude_settings_arg(&config);
        assert!(
            value.ends_with("claude-settings-fast-ultracode.json"),
            "host launches must pass a settings file path, got {value}"
        );
        assert_eq!(
            std::fs::read_to_string(&value).expect("settings file written"),
            r#"{"fastMode":true,"ultracode":true}"#
        );
        assert!(!config.env_vars.contains_key("CLAUDE_CODE_EFFORT_LEVEL"));
    }

    #[test]
    fn materialize_claude_settings_file_writes_content_keyed_file() {
        let temp = tempfile::tempdir().expect("tempdir");
        let json = r#"{"fastMode":true,"ultracode":true}"#;

        let path = materialize_claude_settings_file(temp.path(), true, true, json)
            .expect("settings file materializes");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("claude-settings-fast-ultracode.json")
        );
        assert_eq!(std::fs::read_to_string(&path).expect("read back"), json);
        // Re-materializing the same toggles is idempotent.
        let again = materialize_claude_settings_file(temp.path(), true, true, json)
            .expect("settings file rewrites");
        assert_eq!(again, path);
    }

    #[test]
    fn build_codex_ultracode_does_not_emit_reasoning_effort() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .reasoning_level("ultracode")
            .build();

        // ultracode is Claude-only; Codex must not receive it as an effort.
        assert!(!config
            .args
            .iter()
            .any(|arg| arg == "model_reasoning_effort=ultracode"));
        assert!(!config
            .args
            .windows(2)
            .any(|pair| pair[0] == "-c" && pair[1].starts_with("model_reasoning_effort=")));
    }

    #[test]
    fn build_codex_with_xhigh_still_emits_reasoning_effort() {
        // Regression guard: the ultracode skip must not affect valid Codex levels.
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .reasoning_level("xhigh")
            .build();

        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == "model_reasoning_effort=xhigh"));
    }

    #[test]
    fn build_codex_fast_mode() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .fast_mode(true)
            .version("0.113.0")
            .build();

        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == "service_tier=fast"));
        assert!(!config.args.contains(&"--full-auto".to_string()));
        assert!(config.fast_mode);
        assert!(config.codex_fast_mode);
        assert!(!config.skip_permissions);
    }

    #[test]
    fn build_codex_skip_permissions_adds_yolo() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .skip_permissions(true)
            .build();

        assert!(config.args.contains(&"--yolo".to_string()));
        assert!(config.skip_permissions);
    }

    #[test]
    fn build_codex_resume_with_id_uses_resume_subcommand() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .session_mode(SessionMode::Resume)
            .resume_session_id("sess-123")
            .build();

        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "resume" && pair[1] == "sess-123"));
    }

    #[test]
    fn build_codex_resume_without_id_opens_picker() {
        // SPEC-2014 2026-05-18 amendment FR-B / SC-A:
        // Codex Execution Mode Resume (no session id) must produce a bare
        // `codex resume` so the interactive picker opens. `--last` is reserved
        // for Continue mode only.
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .session_mode(SessionMode::Resume)
            .build();

        let resume_index = config
            .args
            .iter()
            .position(|arg| arg == "resume")
            .expect("codex args must contain the `resume` subcommand");
        assert_ne!(
            config.args.get(resume_index + 1).map(String::as_str),
            Some("--last"),
            "Execution Mode Resume must not append --last; that is Continue's role"
        );
        assert!(
            !config.args.contains(&"--last".to_string()),
            "no --last anywhere when SessionMode::Resume has no resume_session_id"
        );
    }

    #[test]
    fn build_codex_continue_uses_last_session() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .session_mode(SessionMode::Continue)
            .build();

        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "resume" && pair[1] == "--last"));
    }

    #[test]
    fn build_gemini_skip_permissions_adds_yolo() {
        let config = AgentLaunchBuilder::new(AgentId::Gemini)
            .skip_permissions(true)
            .build();

        assert!(config.args.contains(&"--yolo".to_string()));
        assert!(config.skip_permissions);
    }

    #[test]
    fn build_antigravity_maps_model_skip_permissions_and_resume_id() {
        let agent_id = crate::types::resolve_agent_id("agy").expect("Antigravity must resolve");
        let config = AgentLaunchBuilder::new(agent_id)
            .model("gemini-3.5-pro")
            .skip_permissions(true)
            .session_mode(SessionMode::Resume)
            .resume_session_id("conversation-123")
            .build();

        assert_eq!(config.command, "agy");
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--model" && pair[1] == "gemini-3.5-pro"));
        assert!(config
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
        assert!(!config.args.contains(&"--yolo".to_string()));
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--conversation" && pair[1] == "conversation-123"));
    }

    #[test]
    fn build_antigravity_continue_uses_continue_flag() {
        let agent_id = crate::types::resolve_agent_id("antigravity-cli")
            .expect("Antigravity alias must resolve");
        let config = AgentLaunchBuilder::new(agent_id)
            .session_mode(SessionMode::Continue)
            .build();

        assert_eq!(config.command, "agy");
        assert!(config.args.contains(&"--continue".to_string()));
    }

    #[test]
    fn build_copilot_skip_permissions_adds_yolo() {
        let config = AgentLaunchBuilder::new(AgentId::Copilot)
            .skip_permissions(true)
            .build();

        assert!(config.args.contains(&"--yolo".to_string()));
        assert!(config.skip_permissions);
    }

    #[test]
    fn build_codex_fast_mode_omits_service_tier_for_older_versions() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .fast_mode(true)
            .version("0.109.0")
            .build();

        assert!(!config
            .args
            .windows(2)
            .any(|pair| pair[0] == "-c" && pair[1] == "service_tier=fast"));
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
    fn build_codex_does_not_enable_hooks_feature_flag_by_default() {
        let config = AgentLaunchBuilder::new(AgentId::Codex).build();

        assert!(!config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--enable" && pair[1] == "codex_hooks"));
        assert!(!config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--disable" && pair[1] == "hooks"));
        assert!(
            !config
                .args
                .iter()
                .any(|arg| arg.contains("bypass_hook_trust")),
            "Codex launch must not bypass hook review globally: {:?}",
            config.args
        );
    }

    #[test]
    fn build_codex_enables_goal_feature_by_default() {
        let config = AgentLaunchBuilder::new(AgentId::Codex).build();

        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--enable" && pair[1] == "goals"));
    }

    #[test]
    fn build_codex_resume_and_continue_keep_goal_feature_enabled() {
        let resume = AgentLaunchBuilder::new(AgentId::Codex)
            .session_mode(SessionMode::Resume)
            .resume_session_id("sess-123")
            .build();
        let continue_last = AgentLaunchBuilder::new(AgentId::Codex)
            .session_mode(SessionMode::Continue)
            .build();

        assert!(resume
            .args
            .windows(2)
            .any(|pair| pair[0] == "--enable" && pair[1] == "goals"));
        assert!(resume
            .args
            .windows(2)
            .any(|pair| pair[0] == "resume" && pair[1] == "sess-123"));
        assert!(continue_last
            .args
            .windows(2)
            .any(|pair| pair[0] == "--enable" && pair[1] == "goals"));
        assert!(continue_last
            .args
            .windows(2)
            .any(|pair| pair[0] == "resume" && pair[1] == "--last"));
    }

    #[test]
    fn build_non_codex_agents_do_not_enable_goal_feature() {
        for agent in [
            AgentId::ClaudeCode,
            AgentId::Gemini,
            AgentId::OpenCode,
            AgentId::OpenClaw,
            AgentId::Hermes,
            AgentId::Copilot,
            AgentId::Custom("custom".into()),
        ] {
            let config = AgentLaunchBuilder::new(agent).build();

            assert!(!config
                .args
                .windows(2)
                .any(|pair| pair[0] == "--enable" && pair[1] == "goals"));
        }
    }

    #[test]
    fn canonical_codex_args_do_not_include_goal_feature_flag() {
        let args = canonical_launch_args(&AgentId::Codex);

        assert_eq!(args, vec!["--no-alt-screen".to_string()]);
        assert!(!args
            .windows(2)
            .any(|pair| pair[0] == "--enable" && pair[1] == "goals"));
    }

    #[test]
    fn build_codex_disables_alternate_screen() {
        let config = AgentLaunchBuilder::new(AgentId::Codex).build();

        assert!(
            config.args.contains(&"--no-alt-screen".to_string()),
            "Codex should run inline so the PTY emits row scrollback instead of full-screen redraw-only history: {:?}",
            config.args
        );
    }

    #[test]
    fn normalize_launch_args_keeps_npx_runner_prefix_before_inserting_no_alt_screen() {
        let mut args = vec![
            "--yes".to_string(),
            "@openai/codex@latest".to_string(),
            "resume".to_string(),
            "sess-123".to_string(),
        ];

        normalize_launch_args(&AgentId::Codex, "/opt/homebrew/bin/npx", &mut args);

        assert_eq!(
            args,
            vec![
                "--yes".to_string(),
                "@openai/codex@latest".to_string(),
                "--no-alt-screen".to_string(),
                "resume".to_string(),
                "sess-123".to_string(),
            ]
        );
    }

    #[test]
    fn normalize_launch_args_ignores_non_codex_command_for_codex_agent() {
        let mut args = vec![
            "-c".to_string(),
            "printf test".to_string(),
            "sh".to_string(),
        ];

        normalize_launch_args(&AgentId::Codex, "/bin/sh", &mut args);

        assert_eq!(
            args,
            vec![
                "-c".to_string(),
                "printf test".to_string(),
                "sh".to_string(),
            ]
        );
    }

    #[test]
    fn build_codex_adds_runtime_namespace_as_writable_root() {
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .env(
                "GWT_SESSION_RUNTIME_PATH",
                "/Users/akiojin/.gwt/sessions/runtime/36610/session-123.json",
            )
            .build();

        assert!(config.args.windows(2).any(|pair| {
            pair[0] == "--add-dir" && pair[1] == "/Users/akiojin/.gwt/sessions/runtime/36610"
        }));
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
            .model("gemini-3-flash-preview")
            .build();

        assert_eq!(config.command, "gemini");
        assert!(config.args.contains(&"--model".to_string()));
        assert!(config.args.contains(&"gemini-3-flash-preview".to_string()));
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
            config
                .env_vars
                .get("CLAUDE_CODE_NO_FLICKER")
                .map(String::as_str),
            Some("0"),
            "Claude Code launches must default to the classic renderer"
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
        assert!(spec_arg
            .unwrap()
            .contains("@anthropic-ai/claude-code@latest"));
    }

    #[test]
    fn resolve_runner_latest_uses_official_gemini_package() {
        let runner = resolve_runner(&AgentId::Gemini, "latest");
        assert!(!runner.executable.is_empty());
        let spec_arg = runner.base_args.iter().find(|a| a.contains('@'));
        assert_eq!(
            spec_arg.map(String::as_str),
            Some("@google/gemini-cli@latest")
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
        // OpenClaw still has no npm package, so a versioned request must fall
        // back to the direct command rather than a package runner.
        let runner = resolve_runner(&AgentId::OpenClaw, "latest");
        assert_eq!(runner.executable, "openclaw");
        assert!(runner.base_args.is_empty());
    }

    #[test]
    fn build_opencode_sets_model_and_project_config_dir() {
        let config = AgentLaunchBuilder::new(AgentId::OpenCode)
            .working_dir("/tmp/project")
            .model("anthropic/claude-sonnet-4-5")
            .build();

        assert_eq!(config.command, "opencode");
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--model" && pair[1] == "anthropic/claude-sonnet-4-5"));
        assert_eq!(
            config.env_vars.get("OPENCODE_CONFIG_DIR"),
            Some(&project_relative_path(".gwt/opencode"))
        );
    }

    // SPEC-3151 FR-006 / AS-4: OpenCode resume maps a concrete session id to
    // `--session <id>`, while Continue (or Resume without an id) uses
    // `--continue`. OpenCode has no interactive resume picker, so resume-picker
    // support stays out of scope.
    #[test]
    fn build_opencode_resume_passes_session_id() {
        let config = AgentLaunchBuilder::new(AgentId::OpenCode)
            .working_dir("/tmp/project")
            .session_mode(SessionMode::Resume)
            .resume_session_id("sess-9")
            .build();

        assert_eq!(config.command, "opencode");
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--session" && pair[1] == "sess-9"));
        assert!(!config.args.contains(&"--continue".to_string()));
    }

    #[test]
    fn build_opencode_continue_uses_continue_flag() {
        let config = AgentLaunchBuilder::new(AgentId::OpenCode)
            .working_dir("/tmp/project")
            .session_mode(SessionMode::Continue)
            .build();

        assert!(config.args.contains(&"--continue".to_string()));
    }

    #[test]
    fn build_opencode_resume_without_session_id_falls_back_to_continue() {
        let config = AgentLaunchBuilder::new(AgentId::OpenCode)
            .working_dir("/tmp/project")
            .session_mode(SessionMode::Resume)
            .build();

        assert!(config.args.contains(&"--continue".to_string()));
        assert!(!config.args.contains(&"--session".to_string()));
    }

    // SPEC-3151 FR-005: OpenCode has no skip-permissions CLI flag; non-interactive
    // operation is controlled by the opencode.json `permission` config. When a
    // launch opts into skip_permissions, gwt layers a permissive config overlay
    // via OPENCODE_CONFIG, faithfully honoring the per-launch toggle (parity with
    // Codex `--yolo` / Claude `--dangerously-skip-permissions`).
    #[test]
    fn build_opencode_skip_permissions_sets_config_overlay() {
        let config = AgentLaunchBuilder::new(AgentId::OpenCode)
            .working_dir("/tmp/project")
            .skip_permissions(true)
            .build();

        assert_eq!(
            config.env_vars.get("OPENCODE_CONFIG"),
            Some(&project_relative_path(
                ".gwt/opencode/skip-permissions.json"
            ))
        );
    }

    #[test]
    fn build_opencode_without_skip_permissions_omits_config_overlay() {
        let config = AgentLaunchBuilder::new(AgentId::OpenCode)
            .working_dir("/tmp/project")
            .build();

        assert!(!config.env_vars.contains_key("OPENCODE_CONFIG"));
    }

    #[test]
    fn build_openclaw_uses_local_tui_and_gwt_config_path() {
        let config = AgentLaunchBuilder::new(AgentId::OpenClaw)
            .working_dir("/tmp/project")
            .reasoning_level("high")
            .session_mode(SessionMode::Resume)
            .resume_session_id("session-123")
            .build();

        assert_eq!(config.command, "openclaw");
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "tui" && pair[1] == "--local"));
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--session" && pair[1] == "session-123"));
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--thinking" && pair[1] == "high"));
        assert_eq!(
            config.env_vars.get("OPENCLAW_CONFIG_PATH"),
            Some(&project_relative_path(".gwt/openclaw/openclaw.json"))
        );
    }

    #[test]
    fn build_hermes_accepts_hooks_and_maps_launch_controls() {
        let config = AgentLaunchBuilder::new(AgentId::Hermes)
            .working_dir("/tmp/project")
            .model("openrouter/anthropic/claude-sonnet-4")
            .skip_permissions(true)
            .session_mode(SessionMode::Continue)
            .build();

        assert_eq!(config.command, "hermes");
        assert!(config.args.contains(&"chat".to_string()));
        assert!(config.args.contains(&"--accept-hooks".to_string()));
        assert!(config.args.contains(&"--pass-session-id".to_string()));
        assert!(config.args.contains(&"--continue".to_string()));
        assert!(config.args.contains(&"--yolo".to_string()));
        assert!(config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--model" && pair[1] == "openrouter/anthropic/claude-sonnet-4"));
        assert_eq!(
            config.env_vars.get("HERMES_HOME"),
            Some(&project_relative_path(".gwt/hermes"))
        );
        assert_eq!(
            config.env_vars.get("HERMES_ACCEPT_HOOKS"),
            Some(&"1".to_string())
        );
    }

    #[test]
    fn build_hermes_maps_provider_profile_and_advanced_options() {
        let config = AgentLaunchBuilder::new(AgentId::Hermes)
            .working_dir("/tmp/project")
            .provider("openrouter")
            .model("anthropic/claude-sonnet-4")
            .profile("work")
            .toolsets("fs,web")
            .skills("gwt-build-spec")
            .max_turns(40)
            .safe_mode(true)
            .build();

        let has_pair =
            |flag: &str, val: &str| config.args.windows(2).any(|p| p[0] == flag && p[1] == val);
        assert!(has_pair("--provider", "openrouter"));
        assert!(has_pair("--model", "anthropic/claude-sonnet-4"));
        assert!(has_pair("--profile", "work"));
        assert!(has_pair("--toolsets", "fs,web"));
        assert!(has_pair("--skills", "gwt-build-spec"));
        assert!(has_pair("--max-turns", "40"));
        assert!(config.args.contains(&"--safe-mode".to_string()));
    }

    #[test]
    fn build_hermes_omits_unset_optional_flags() {
        let config = AgentLaunchBuilder::new(AgentId::Hermes)
            .working_dir("/tmp/project")
            .build();

        for flag in [
            "--provider",
            "--profile",
            "--toolsets",
            "--skills",
            "--max-turns",
            "--safe-mode",
        ] {
            assert!(
                !config.args.iter().any(|a| a == flag),
                "unset option must not emit {flag}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_package_runner_candidates_prefer_cmd_variants() {
        // SPEC-1921 FR-080. On Windows we must consult `bunx.cmd` / `npx.cmd`
        // before the POSIX-shim bare-name siblings, because `which::which`
        // returns the bare name first and that file is not spawnable through
        // `CreateProcess` on its own.
        let candidates = package_runner_candidates();
        let names: Vec<&str> = candidates.iter().map(|(name, _)| *name).collect();

        assert_eq!(names, vec!["bunx.cmd", "bunx", "npx.cmd", "npx"]);

        let needs_yes: Vec<bool> = candidates.iter().map(|(_, yes)| *yes).collect();
        assert_eq!(needs_yes, vec![false, false, true, true]);
    }

    #[cfg(not(windows))]
    #[test]
    fn nonwindows_package_runner_candidates_use_bare_names() {
        let candidates = package_runner_candidates();
        let names: Vec<&str> = candidates.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, vec!["bunx", "npx"]);

        let needs_yes: Vec<bool> = candidates.iter().map(|(_, yes)| *yes).collect();
        assert_eq!(needs_yes, vec![false, true]);
    }

    #[cfg(all(not(windows), unix))]
    #[test]
    fn package_runner_detection_uses_hydrated_host_env_path() {
        use std::os::unix::fs::PermissionsExt;

        let home = tempfile::tempdir().expect("home");
        let bin = home.path().join(".bun/bin");
        std::fs::create_dir_all(&bin).expect("create bin");
        let bunx = bin.join("bunx");
        std::fs::write(&bunx, "#!/bin/sh\nexit 0\n").expect("write bunx");
        let mut permissions = std::fs::metadata(&bunx)
            .expect("bunx metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&bunx, permissions).expect("chmod bunx");

        let env = HashMap::from([
            (
                "PATH".to_string(),
                "/usr/bin:/bin:/usr/sbin:/sbin".to_string(),
            ),
            ("HOME".to_string(), home.path().display().to_string()),
        ]);

        let (executable, needs_yes) = find_bunx_or_npx_with_env(&env);

        assert_eq!(PathBuf::from(executable), bunx);
        assert!(!needs_yes);
    }

    #[cfg(all(not(windows), unix))]
    fn write_test_runner(path: &Path) {
        use std::os::unix::fs::PermissionsExt;

        std::fs::write(path, "#!/bin/sh\nexit 0\n").expect("write runner");
        let mut permissions = std::fs::metadata(path)
            .expect("runner metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("chmod runner");
    }

    #[cfg(all(not(windows), unix))]
    #[test]
    fn claude_latest_prefers_npx_over_bunx_on_nonwindows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bunx = temp.path().join("bunx");
        let npx = temp.path().join("npx");
        write_test_runner(&bunx);
        write_test_runner(&npx);
        let env = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);

        let runner = resolve_runner_with_env(&AgentId::ClaudeCode, "latest", &env);

        assert_eq!(PathBuf::from(runner.executable), npx);
        assert_eq!(
            runner.base_args,
            vec![
                "--yes".to_string(),
                "@anthropic-ai/claude-code@latest".to_string(),
            ],
        );
    }

    #[cfg(all(not(windows), unix))]
    #[test]
    fn claude_latest_falls_back_to_bunx_when_npx_is_missing_on_nonwindows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bunx = temp.path().join("bunx");
        write_test_runner(&bunx);
        let path = temp.path().display().to_string();
        let cwd = std::env::current_dir().expect("cwd");

        let (executable, needs_yes) = find_package_runner_in_path(
            package_runner_candidates_for_agent(&AgentId::ClaudeCode),
            Some(path.as_str()),
            &cwd,
        )
        .expect("runner");

        assert_eq!(PathBuf::from(executable), bunx);
        assert!(!needs_yes);
    }

    #[cfg(all(not(windows), unix))]
    #[test]
    fn codex_latest_keeps_bunx_first_on_nonwindows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bunx = temp.path().join("bunx");
        let npx = temp.path().join("npx");
        write_test_runner(&bunx);
        write_test_runner(&npx);
        let env = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);

        let runner = resolve_runner_with_env(&AgentId::Codex, "latest", &env);

        assert_eq!(PathBuf::from(runner.executable), bunx);
        assert_eq!(runner.base_args, vec!["@openai/codex@latest".to_string()]);
    }

    // SPEC-3151 FR-001/FR-002: OpenCode launches through the `opencode-ai` npm
    // package runner just like Codex/Claude Code. Per the SPEC decision the
    // runner keeps bunx-first ordering (OpenCode is not on the npx-preference
    // list), so a versioned launch resolves bunx + `opencode-ai@<version>`.
    #[cfg(all(not(windows), unix))]
    #[test]
    fn opencode_latest_keeps_bunx_first_on_nonwindows() {
        let temp = tempfile::tempdir().expect("tempdir");
        let bunx = temp.path().join("bunx");
        let npx = temp.path().join("npx");
        write_test_runner(&bunx);
        write_test_runner(&npx);
        let env = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);

        let runner = resolve_runner_with_env(&AgentId::OpenCode, "latest", &env);

        assert_eq!(PathBuf::from(runner.executable), bunx);
        assert_eq!(runner.base_args, vec!["opencode-ai@latest".to_string()]);
    }

    // Issue #2981: the host `bunx`→`npx` fallback must resolve a Windows-spawnable
    // executable. `find_package_runner_in_path` is the shared resolver, so given
    // an npx-only candidate list it must prefer the `.cmd` variant ahead of the
    // bare POSIX shim when both are present on PATH (SPEC-1921 FR-080).
    #[cfg(all(not(windows), unix))]
    #[test]
    fn npx_fallback_resolution_prefers_cmd_variant_when_both_present() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_test_runner(&temp.path().join("npx.cmd"));
        write_test_runner(&temp.path().join("npx"));
        let path = temp.path().display().to_string();
        let cwd = std::env::current_dir().expect("cwd");
        let candidates: &'static [(&'static str, bool)] = &[("npx.cmd", true), ("npx", true)];

        let (executable, needs_yes) =
            find_package_runner_in_path(candidates, Some(path.as_str()), &cwd).expect("npx runner");

        assert_eq!(PathBuf::from(&executable), temp.path().join("npx.cmd"));
        assert!(needs_yes);
    }

    #[cfg(all(not(windows), unix))]
    #[test]
    fn resolve_host_npx_fallback_executable_resolves_npx_on_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        write_test_runner(&temp.path().join("npx"));
        let env = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);

        let resolved = resolve_host_npx_fallback_executable(&env);

        assert_eq!(PathBuf::from(resolved), temp.path().join("npx"));
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_host_npx_fallback_executable_defaults_to_bare_npx_when_absent() {
        // Empty PATH dir: no npx anywhere → canonical bare name is preserved so
        // the launch PTY can still resolve it at spawn time (prior behavior).
        let temp = tempfile::tempdir().expect("tempdir");
        let env = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);

        let resolved = resolve_host_npx_fallback_executable(&env);

        assert_eq!(resolved, "npx");
    }

    #[cfg(windows)]
    #[test]
    fn resolve_host_npx_fallback_executable_prefers_cmd_on_windows() {
        let temp = tempfile::tempdir().expect("tempdir");
        std::fs::write(temp.path().join("npx.cmd"), "@echo off\r\n").expect("write npx.cmd");
        std::fs::write(temp.path().join("npx"), "#!/bin/sh\n").expect("write npx shim");
        let env = HashMap::from([("PATH".to_string(), temp.path().display().to_string())]);

        let resolved = resolve_host_npx_fallback_executable(&env);

        assert!(
            Path::new(&resolved)
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| name.eq_ignore_ascii_case("npx.cmd"))
                .unwrap_or(false),
            "expected npx.cmd, got {resolved}"
        );
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

    #[test]
    fn custom_agent_definition_overrides_display_name_runner_args_and_env() {
        let agent = CustomCodingAgent {
            id: "proxy-agent".to_string(),
            display_name: "Claude Proxy".to_string(),
            agent_type: CustomAgentType::Bunx,
            command: "@anthropic-ai/claude-code@latest".to_string(),
            default_args: vec!["--print".to_string()],
            mode_args: Some(ModeArgs {
                normal: Vec::new(),
                continue_mode: vec!["--continue".to_string()],
                resume: vec!["--resume".to_string()],
            }),
            skip_permissions_args: vec!["--dangerously-skip-permissions".to_string()],
            env: HashMap::from([(
                "ANTHROPIC_BASE_URL".to_string(),
                "http://proxy.local:32768".to_string(),
            )]),
            supports_resume_picker: false,
        };

        let config = AgentLaunchBuilder::new(AgentId::Custom("proxy-agent".into()))
            .custom_agent(agent)
            .session_mode(SessionMode::Continue)
            .skip_permissions(true)
            .build();

        assert_eq!(config.display_name, "Claude Proxy");
        assert!(
            config.command.contains("bunx") || config.command.contains("npx"),
            "custom bunx agent should resolve through a package runner: {}",
            config.command
        );
        assert!(
            config
                .args
                .iter()
                .any(|arg| arg.contains("@anthropic-ai/claude-code@latest")),
            "custom bunx agent must include the package spec: {:?}",
            config.args
        );
        assert!(config.args.contains(&"--print".to_string()));
        assert!(config.args.contains(&"--continue".to_string()));
        assert!(config
            .args
            .contains(&"--dangerously-skip-permissions".to_string()));
        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_BASE_URL")
                .map(String::as_str),
            Some("http://proxy.local:32768")
        );
    }

    #[test]
    fn custom_agent_env_is_merged_into_spawn_env() {
        let mut env = HashMap::new();
        env.insert(
            "ANTHROPIC_BASE_URL".to_string(),
            "http://proxy.local:32768".to_string(),
        );
        env.insert("ANTHROPIC_API_KEY".to_string(), "sk-test".to_string());

        let config = AgentLaunchBuilder::new(AgentId::Custom("my-custom".into()))
            .custom_agent_env(env)
            .build();

        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_BASE_URL")
                .map(String::as_str),
            Some("http://proxy.local:32768")
        );
        assert_eq!(
            config.env_vars.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-test")
        );
    }

    #[test]
    fn custom_agent_env_wins_over_common_env_on_collision() {
        // TERM is set by the Common env layer to xterm-256color.
        // A custom agent env entry for TERM must override it (FR-063 merge order).
        let mut env = HashMap::new();
        env.insert("TERM".to_string(), "xterm-kitty".to_string());

        let config = AgentLaunchBuilder::new(AgentId::Custom("x".into()))
            .custom_agent_env(env)
            .build();

        assert_eq!(
            config.env_vars.get("TERM").map(String::as_str),
            Some("xterm-kitty"),
            "custom env must win over Common env (FR-063 merge order)"
        );
    }

    #[test]
    fn env_override_still_wins_over_custom_agent_env() {
        // env() override is the explicit caller-provided layer and must remain
        // authoritative over preset-seeded custom agent env (FR-063).
        let mut env = HashMap::new();
        env.insert("FOO".to_string(), "from-custom".to_string());

        let config = AgentLaunchBuilder::new(AgentId::Custom("x".into()))
            .custom_agent_env(env)
            .env("FOO", "from-override")
            .build();

        assert_eq!(
            config.env_vars.get("FOO").map(String::as_str),
            Some("from-override"),
            "env_overrides must remain authoritative over custom_agent_env"
        );
    }

    #[test]
    fn custom_agent_env_can_still_override_claude_no_flicker() {
        // If a user somehow applies custom_agent_env to a built-in Claude
        // launch, an explicit value should still override gwt's renderer
        // default.
        let mut env = HashMap::new();
        env.insert("CLAUDE_CODE_NO_FLICKER".to_string(), "1".to_string());

        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .custom_agent_env(env)
            .build();

        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_NO_FLICKER")
                .map(String::as_str),
            Some("1"),
            "custom_agent_env must remain able to pass explicit Claude env"
        );
    }

    #[test]
    fn custom_agent_env_empty_does_not_affect_other_env() {
        // Absent custom_agent_env (default empty HashMap) must not change
        // the env_vars produced for a built-in Claude launch.
        let without = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();
        let with_empty = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .custom_agent_env(HashMap::new())
            .build();
        assert_eq!(without.env_vars, with_empty.env_vars);
    }

    // ---- SPEC-1921 FR-100 (2026-05-18 amendment) Backend Override env injection.

    fn sample_claude_backend() -> crate::backend::AgentBackendProfile {
        crate::backend::AgentBackendProfile {
            id: "lmstudio".into(),
            display_name: "LM Studio".into(),
            base_url: "http://192.168.100.166:32768".into(),
            api_key: "sk-test".into(),
            model: "openai/gpt-oss-20b".into(),
            ..Default::default()
        }
    }

    #[test]
    fn default_claude_launch_without_backend_profile_omits_backend_env() {
        // FR-100 default contract: no backend profile -> upstream stays
        // at Anthropic's default and gwt does not export any of the
        // ANTHROPIC_BASE_URL / ANTHROPIC_API_KEY / model-role env vars.
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode).build();
        for key in [
            "ANTHROPIC_BASE_URL",
            "ANTHROPIC_API_KEY",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_ATTRIBUTION_HEADER",
            "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC",
        ] {
            assert!(
                !config.env_vars.contains_key(key),
                "{key} must not be set when no backend profile is active"
            );
        }
    }

    #[test]
    fn claude_backend_profile_injects_anthropic_env_and_telemetry_off_flags() {
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .backend_profile(sample_claude_backend())
            .build();

        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_BASE_URL")
                .map(String::as_str),
            Some("http://192.168.100.166:32768")
        );
        assert_eq!(
            config.env_vars.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-test")
        );
        // FR-098: `model` fans out to all four role env vars by default.
        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .map(String::as_str),
            Some("openai/gpt-oss-20b")
        );
        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
                .map(String::as_str),
            Some("openai/gpt-oss-20b")
        );
        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                .map(String::as_str),
            Some("openai/gpt-oss-20b")
        );
        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_SUBAGENT_MODEL")
                .map(String::as_str),
            Some("openai/gpt-oss-20b")
        );
        // Telemetry off when proxying to a non-Anthropic upstream.
        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_ATTRIBUTION_HEADER")
                .map(String::as_str),
            Some("0")
        );
        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC")
                .map(String::as_str),
            Some("1")
        );
    }

    #[test]
    fn claude_backend_profile_role_overrides_replace_fan_out() {
        let mut profile = sample_claude_backend();
        profile.haiku_model = Some("haiku-model".into());
        profile.opus_model = Some("opus-model".into());
        profile.sonnet_model = Some("sonnet-model".into());
        profile.subagent_model = Some("sub-model".into());

        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .backend_profile(profile)
            .build();

        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .map(String::as_str),
            Some("haiku-model")
        );
        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_DEFAULT_OPUS_MODEL")
                .map(String::as_str),
            Some("opus-model")
        );
        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_DEFAULT_SONNET_MODEL")
                .map(String::as_str),
            Some("sonnet-model")
        );
        assert_eq!(
            config
                .env_vars
                .get("CLAUDE_CODE_SUBAGENT_MODEL")
                .map(String::as_str),
            Some("sub-model")
        );
    }

    #[test]
    fn env_override_still_wins_over_backend_profile_env() {
        // Explicit caller `env()` override remains the most authoritative
        // layer (FR-063 merge order). Useful for tests and emergency
        // operator-level rerouting.
        let config = AgentLaunchBuilder::new(AgentId::ClaudeCode)
            .backend_profile(sample_claude_backend())
            .env("ANTHROPIC_BASE_URL", "http://emergency-override:80")
            .build();

        assert_eq!(
            config
                .env_vars
                .get("ANTHROPIC_BASE_URL")
                .map(String::as_str),
            Some("http://emergency-override:80")
        );
        // But other backend env vars are still applied.
        assert_eq!(
            config.env_vars.get("ANTHROPIC_API_KEY").map(String::as_str),
            Some("sk-test")
        );
    }

    #[test]
    fn backend_profile_does_not_inject_anthropic_env_for_non_claude_agents() {
        // FR-103 / Phase 63E: Codex backend env is applied via worktree-local
        // CODEX_HOME generation, not through ANTHROPIC_* env vars. Calling
        // backend_profile() on a Codex launch must not accidentally export
        // ANTHROPIC_* env vars (those belong to Claude Code only).
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .backend_profile(sample_claude_backend())
            .build();

        assert!(!config.env_vars.contains_key("ANTHROPIC_BASE_URL"));
        assert!(!config.env_vars.contains_key("ANTHROPIC_API_KEY"));
        assert!(!config.env_vars.contains_key("ANTHROPIC_DEFAULT_OPUS_MODEL"));
        assert!(!config.env_vars.contains_key("CLAUDE_CODE_SUBAGENT_MODEL"));
    }

    // ---- SPEC-1921 FR-103: Codex backend integration tests.

    fn sample_codex_backend(id: &str) -> crate::backend::AgentBackendProfile {
        crate::backend::AgentBackendProfile {
            id: id.into(),
            display_name: "llmlb".into(),
            base_url: "http://127.0.0.1:8080".into(),
            api_key: "sk-codex".into(),
            model: "local/qwen3-coder".into(),
            ..Default::default()
        }
    }

    #[test]
    fn codex_backend_profile_sets_codex_home_and_api_key_env() {
        let tmp = tempfile::tempdir().unwrap();
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir(tmp.path())
            .backend_profile(sample_codex_backend("llmlb"))
            .build();

        let codex_home = config
            .env_vars
            .get("CODEX_HOME")
            .expect("CODEX_HOME must be set when a Codex backend profile is active");
        assert!(
            Path::new(codex_home).ends_with(Path::new(".gwt").join("codex")),
            "got {codex_home}"
        );
        // API key env defaults to the GWT_CODEX_BACKEND_API_KEY_<UPPER> name.
        assert_eq!(
            config
                .env_vars
                .get("GWT_CODEX_BACKEND_API_KEY_LLMLB")
                .map(String::as_str),
            Some("sk-codex")
        );

        // Generated config.toml exists and contains the expected provider id.
        let body =
            std::fs::read_to_string(Path::new(codex_home).join("config.toml")).expect("read");
        assert!(body.contains("model_provider = \"gwt-llmlb\""));
        assert!(body.contains("base_url = \"http://127.0.0.1:8080\""));
    }

    #[test]
    fn codex_backend_profile_honors_custom_env_key() {
        let tmp = tempfile::tempdir().unwrap();
        let mut profile = sample_codex_backend("llmlb");
        profile.env_key = Some("MY_TEAM_CODEX_KEY".into());
        let config = AgentLaunchBuilder::new(AgentId::Codex)
            .working_dir(tmp.path())
            .backend_profile(profile)
            .build();

        assert_eq!(
            config.env_vars.get("MY_TEAM_CODEX_KEY").map(String::as_str),
            Some("sk-codex")
        );
        assert!(
            !config
                .env_vars
                .contains_key("GWT_CODEX_BACKEND_API_KEY_LLMLB"),
            "explicit env_key must replace the default name"
        );
    }

    #[test]
    fn codex_launch_without_backend_profile_does_not_set_codex_home() {
        let config = AgentLaunchBuilder::new(AgentId::Codex).build();
        assert!(!config.env_vars.contains_key("CODEX_HOME"));
        assert!(!config
            .env_vars
            .keys()
            .any(|k| k.starts_with("GWT_CODEX_BACKEND_API_KEY_")));
    }
}
