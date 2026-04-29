use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowPreset {
    Shell,
    Claude,
    Codex,
    Agent,
    FileTree,
    Branches,
    Settings,
    Memo,
    Profile,
    Logs,
    Issue,
    Spec,
    Board,
    Pr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowSurface {
    Terminal,
    FileTree,
    Branches,
    Memo,
    Profile,
    Board,
    Logs,
    Knowledge,
    Mock,
}

impl WindowSurface {
    /// SPEC-2008 FR-036: kebab-case identifier shared with the JS
    /// `presetSurface()` mapping. Whenever a variant is added, the JS
    /// side and the contract test in `embedded_web.rs` must move in
    /// lockstep.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
            Self::FileTree => "file-tree",
            Self::Branches => "branches",
            Self::Memo => "memo",
            Self::Profile => "profile",
            Self::Board => "board",
            Self::Logs => "logs",
            Self::Knowledge => "knowledge",
            Self::Mock => "mock",
        }
    }
}

impl WindowPreset {
    pub const ALL: [WindowPreset; 13] = [
        WindowPreset::Shell,
        WindowPreset::Claude,
        WindowPreset::Codex,
        WindowPreset::FileTree,
        WindowPreset::Branches,
        WindowPreset::Settings,
        WindowPreset::Memo,
        WindowPreset::Profile,
        WindowPreset::Logs,
        WindowPreset::Issue,
        WindowPreset::Spec,
        WindowPreset::Board,
        WindowPreset::Pr,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Shell => "Shell",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
            Self::Agent => "Agent",
            Self::FileTree => "File Tree",
            Self::Branches => "Branches",
            Self::Settings => "Settings",
            Self::Memo => "Memo",
            Self::Profile => "Profile",
            Self::Logs => "Logs",
            Self::Issue => "Issue",
            Self::Spec => "SPEC",
            Self::Board => "Board",
            Self::Pr => "PR",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Shell => "Open a standard shell terminal",
            Self::Claude => "Start the Claude CLI when available",
            Self::Codex => "Start the Codex CLI when available",
            Self::Agent => "Start a wizard-launched agent terminal",
            Self::FileTree => "Browse repository files in a read-only tree",
            Self::Branches => "Browse repository branches and launch agents",
            Self::Settings => "Manage custom agents and launch presets",
            Self::Memo => "Capture repo-scoped notes and pinned follow-ups",
            Self::Profile => "Manage env profiles, overrides, and merged preview",
            Self::Logs => "Placeholder logs surface",
            Self::Issue => "Placeholder issue surface",
            Self::Spec => "Placeholder SPEC surface",
            Self::Board => "Placeholder board surface",
            Self::Pr => "Placeholder PR surface",
        }
    }

    pub fn id_prefix(self) -> &'static str {
        match self {
            Self::Shell => "shell",
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Agent => "agent",
            Self::FileTree => "file-tree",
            Self::Branches => "branches",
            Self::Settings => "settings",
            Self::Memo => "memo",
            Self::Profile => "profile",
            Self::Logs => "logs",
            Self::Issue => "issue",
            Self::Spec => "spec",
            Self::Board => "board",
            Self::Pr => "pr",
        }
    }

    pub fn surface(self) -> WindowSurface {
        match self {
            Self::Shell | Self::Claude | Self::Codex | Self::Agent => WindowSurface::Terminal,
            Self::FileTree => WindowSurface::FileTree,
            Self::Branches => WindowSurface::Branches,
            Self::Memo => WindowSurface::Memo,
            Self::Profile => WindowSurface::Profile,
            Self::Logs => WindowSurface::Logs,
            Self::Board => WindowSurface::Board,
            Self::Issue | Self::Spec | Self::Pr => WindowSurface::Knowledge,
            Self::Settings => WindowSurface::Mock,
        }
    }

    pub fn requires_process(self) -> bool {
        matches!(self.surface(), WindowSurface::Terminal)
    }

    pub fn default_size(self) -> (f64, f64) {
        match self.surface() {
            WindowSurface::Terminal => (720.0, 420.0),
            WindowSurface::FileTree => (420.0, 520.0),
            WindowSurface::Branches => (520.0, 420.0),
            WindowSurface::Knowledge => (560.0, 420.0),
            WindowSurface::Memo | WindowSurface::Profile | WindowSurface::Logs => (560.0, 420.0),
            WindowSurface::Board => (520.0, 480.0),
            WindowSurface::Mock => (420.0, 300.0),
        }
    }

    pub fn command_name(self) -> Option<&'static str> {
        match self {
            Self::Shell => None,
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
            Self::Agent => None,
            Self::FileTree
            | Self::Branches
            | Self::Settings
            | Self::Memo
            | Self::Profile
            | Self::Logs
            | Self::Issue
            | Self::Spec
            | Self::Board
            | Self::Pr => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellProgram {
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub title: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PresetResolveError {
    #[error("shell program could not be resolved")]
    ShellNotFound,
    #[error("required command is not available: {command}")]
    CommandNotFound { command: String },
    #[error("preset is not launchable as a process: {preset}")]
    NotLaunchable { preset: String },
}

pub fn detect_shell_program() -> Result<ShellProgram, PresetResolveError> {
    detect_shell_program_with(
        std::env::var("SHELL").ok().as_deref(),
        cfg!(windows),
        runtime_command_exists,
    )
}

pub fn detect_shell_program_with<F>(
    env_shell: Option<&str>,
    is_windows: bool,
    command_exists: F,
) -> Result<ShellProgram, PresetResolveError>
where
    F: Fn(&str) -> bool,
{
    if !is_windows {
        if let Some(shell) = env_shell.map(str::trim).filter(|shell| !shell.is_empty()) {
            if command_exists(shell) {
                return Ok(ShellProgram {
                    command: shell.to_string(),
                    args: login_shell_args(shell),
                });
            }
        }
    }

    let candidates: &[&str] = if is_windows {
        &["pwsh", "powershell", "cmd"]
    } else {
        &["/bin/zsh", "/bin/bash", "/bin/sh", "zsh", "bash", "sh"]
    };

    for candidate in candidates {
        if command_exists(candidate) {
            return Ok(ShellProgram {
                command: (*candidate).to_string(),
                args: login_shell_args(candidate),
            });
        }
    }

    Err(PresetResolveError::ShellNotFound)
}

fn login_shell_args(shell: &str) -> Vec<String> {
    let name = Path::new(shell)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(shell)
        .to_ascii_lowercase();
    match name.as_str() {
        "fish" => vec!["--login".to_string()],
        "bash" | "dash" | "ksh" | "sh" | "zsh" => vec!["-l".to_string()],
        _ => Vec::new(),
    }
}

pub fn resolve_launch_spec(preset: WindowPreset) -> Result<LaunchSpec, PresetResolveError> {
    let shell = detect_shell_program()?;
    resolve_launch_spec_with(preset, &shell, runtime_command_exists)
}

pub fn resolve_launch_spec_with<F>(
    preset: WindowPreset,
    shell: &ShellProgram,
    command_exists: F,
) -> Result<LaunchSpec, PresetResolveError>
where
    F: Fn(&str) -> bool,
{
    if !preset.requires_process() {
        return Err(PresetResolveError::NotLaunchable {
            preset: preset.title().to_string(),
        });
    }

    match preset {
        WindowPreset::Shell => Ok(LaunchSpec {
            title: WindowPreset::Shell.title().to_string(),
            command: shell.command.clone(),
            args: shell.args.clone(),
        }),
        WindowPreset::Claude | WindowPreset::Codex => {
            let command = preset.command_name().expect("command preset");
            if !command_exists(command) {
                return Err(PresetResolveError::CommandNotFound {
                    command: command.to_string(),
                });
            }
            let agent_id = match preset {
                WindowPreset::Codex => gwt_agent::AgentId::Codex,
                WindowPreset::Claude => gwt_agent::AgentId::ClaudeCode,
                _ => unreachable!("outer match narrows to Claude/Codex"),
            };
            Ok(LaunchSpec {
                title: preset.title().to_string(),
                command: command.to_string(),
                args: gwt_agent::canonical_launch_args(&agent_id),
            })
        }
        WindowPreset::Agent
        | WindowPreset::FileTree
        | WindowPreset::Branches
        | WindowPreset::Settings
        | WindowPreset::Memo
        | WindowPreset::Profile
        | WindowPreset::Logs
        | WindowPreset::Issue
        | WindowPreset::Spec
        | WindowPreset::Board
        | WindowPreset::Pr => unreachable!("non-process presets are rejected above"),
    }
}

fn runtime_command_exists(command: &str) -> bool {
    if command.contains(std::path::MAIN_SEPARATOR) || Path::new(command).is_absolute() {
        Path::new(command).exists()
    } else {
        gwt_core::process::command_exists(command)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_tree_preset_is_non_process_file_tree_surface() {
        assert_eq!(WindowPreset::FileTree.surface(), WindowSurface::FileTree);
        assert!(!WindowPreset::FileTree.requires_process());
        assert_eq!(WindowPreset::FileTree.title(), "File Tree");
    }

    #[test]
    fn branches_preset_is_non_process_branch_surface() {
        assert_eq!(WindowPreset::Branches.surface(), WindowSurface::Branches);
        assert!(!WindowPreset::Branches.requires_process());
        assert_eq!(WindowPreset::Branches.title(), "Branches");
    }

    #[test]
    fn settings_preset_is_mock_surface() {
        assert_eq!(WindowPreset::Settings.surface(), WindowSurface::Mock);
        assert!(!WindowPreset::Settings.requires_process());
    }

    #[test]
    fn resolve_shell_preset_uses_supplied_shell_program() {
        let shell = ShellProgram {
            command: "/bin/zsh".to_string(),
            args: vec![],
        };
        let result = resolve_launch_spec_with(WindowPreset::Shell, &shell, |_| true)
            .expect("shell preset should resolve");
        assert!(
            !result.command.is_empty(),
            "shell preset should expose a concrete command"
        );
        assert_eq!(result.title, "Shell");
        assert_eq!(result.command, "/bin/zsh");
    }

    #[test]
    fn resolve_claude_preset_uses_claude_command_when_available() {
        let shell = ShellProgram {
            command: "/bin/zsh".to_string(),
            args: vec![],
        };
        let result =
            resolve_launch_spec_with(WindowPreset::Claude, &shell, |command| command == "claude")
                .expect("claude preset should resolve");
        assert_eq!(result.title, "Claude");
        assert_eq!(result.command, "claude");
        assert!(result.args.is_empty());
    }

    #[test]
    fn shell_detection_prefers_env_shell_on_unix() {
        let shell =
            detect_shell_program_with(Some("/bin/fish"), false, |_| true).expect("shell exists");
        assert_eq!(shell.command, "/bin/fish");
    }

    #[test]
    fn shell_detection_uses_login_args_for_known_unix_shells() {
        let zsh = detect_shell_program_with(Some("/bin/zsh"), false, |_| true).expect("zsh exists");
        assert_eq!(zsh.args, vec!["-l".to_string()]);

        let bash =
            detect_shell_program_with(Some("/bin/bash"), false, |_| true).expect("bash exists");
        assert_eq!(bash.args, vec!["-l".to_string()]);

        let fish = detect_shell_program_with(Some("/opt/homebrew/bin/fish"), false, |_| true)
            .expect("fish exists");
        assert_eq!(fish.args, vec!["--login".to_string()]);

        let custom = detect_shell_program_with(Some("/opt/tools/custom-shell"), false, |_| true)
            .expect("custom shell exists");
        assert!(
            custom.args.is_empty(),
            "unknown shells should not receive guessed login flags"
        );
    }

    #[test]
    fn shell_detection_prefers_pwsh_on_windows() {
        let shell = detect_shell_program_with(None, true, |command| command == "pwsh")
            .expect("windows shell should resolve");
        assert_eq!(shell.command, "pwsh");
    }

    #[test]
    fn resolve_codex_preset_errors_when_command_missing() {
        let shell = ShellProgram {
            command: "/bin/zsh".to_string(),
            args: vec![],
        };
        let error = resolve_launch_spec_with(WindowPreset::Codex, &shell, |_| false)
            .expect_err("codex preset should fail when command is unavailable");
        assert_eq!(
            error,
            PresetResolveError::CommandNotFound {
                command: "codex".to_string()
            }
        );
    }

    #[test]
    fn preset_metadata_exposes_titles_prefixes_and_defaults() {
        assert_eq!(WindowPreset::ALL.len(), 13);
        assert_eq!(WindowPreset::Issue.title(), "Issue");
        assert_eq!(WindowPreset::Spec.title(), "SPEC");
        assert_eq!(WindowPreset::Pr.title(), "PR");
        assert_eq!(WindowPreset::Board.id_prefix(), "board");
        assert_eq!(WindowPreset::Agent.id_prefix(), "agent");
        assert_eq!(WindowPreset::Memo.surface(), WindowSurface::Memo);
        assert_eq!(WindowPreset::Profile.surface(), WindowSurface::Profile);
        assert_eq!(WindowPreset::Board.surface(), WindowSurface::Board);
        assert_eq!(WindowPreset::Logs.surface(), WindowSurface::Logs);
        assert_eq!(WindowPreset::Issue.surface(), WindowSurface::Knowledge);
        assert_eq!(WindowPreset::Spec.surface(), WindowSurface::Knowledge);
        assert_eq!(WindowPreset::Pr.surface(), WindowSurface::Knowledge);
        assert_eq!(WindowPreset::Settings.surface(), WindowSurface::Mock);
        assert_eq!(WindowPreset::Logs.default_size(), (560.0, 420.0));
        assert_eq!(WindowPreset::Shell.default_size(), (720.0, 420.0));
        assert_eq!(WindowPreset::FileTree.default_size(), (420.0, 520.0));
        assert_eq!(WindowPreset::Branches.default_size(), (520.0, 420.0));
        assert_eq!(WindowPreset::Shell.command_name(), None);
        assert_eq!(WindowPreset::Claude.command_name(), Some("claude"));
        assert_eq!(WindowPreset::Codex.command_name(), Some("codex"));
        assert!(!WindowPreset::Issue.requires_process());
        assert_eq!(
            WindowPreset::Profile.subtitle(),
            "Manage env profiles, overrides, and merged preview"
        );
    }

    #[test]
    fn shell_detection_falls_back_and_errors_when_no_shell_exists() {
        let shell = detect_shell_program_with(Some("/bin/fish"), false, |candidate| {
            candidate == "/bin/bash"
        })
        .expect("fallback shell");
        assert_eq!(shell.command, "/bin/bash");
        assert_eq!(shell.args, vec!["-l".to_string()]);

        let error = detect_shell_program_with(None, false, |_| false)
            .expect_err("missing shell should error");
        assert_eq!(error, PresetResolveError::ShellNotFound);
    }

    #[test]
    fn resolve_launch_spec_rejects_non_process_presets_and_runtime_command_handles_paths() {
        let shell = ShellProgram {
            command: "/bin/zsh".to_string(),
            args: vec!["-l".to_string()],
        };
        let error = resolve_launch_spec_with(WindowPreset::Issue, &shell, |_| true)
            .expect_err("issue preset is not launchable");
        assert_eq!(
            error,
            PresetResolveError::NotLaunchable {
                preset: "Issue".to_string()
            }
        );

        let temp = tempfile::tempdir().expect("tempdir");
        let tool = temp
            .path()
            .join(if cfg!(windows) { "tool.cmd" } else { "tool" });
        std::fs::write(&tool, b"echo").expect("write tool");
        assert!(runtime_command_exists(&tool.display().to_string()));
        assert!(!runtime_command_exists(
            &temp.path().join("missing").display().to_string()
        ));
    }

    #[test]
    fn window_preset_catalog_covers_all_titles_surfaces_and_commands() {
        let mut prefixes = std::collections::HashSet::new();

        for preset in WindowPreset::ALL {
            assert!(
                prefixes.insert(preset.id_prefix()),
                "duplicate id prefix for {preset:?}"
            );
            assert!(!preset.title().is_empty());
            assert!(!preset.subtitle().is_empty());

            let (width, height) = preset.default_size();
            assert!(width >= 420.0);
            assert!(height >= 300.0);

            match preset {
                WindowPreset::Shell => {
                    assert_eq!(preset.surface(), WindowSurface::Terminal);
                    assert!(preset.requires_process());
                    assert_eq!(preset.command_name(), None);
                }
                WindowPreset::Claude => {
                    assert_eq!(preset.surface(), WindowSurface::Terminal);
                    assert!(preset.requires_process());
                    assert_eq!(preset.command_name(), Some("claude"));
                }
                WindowPreset::Codex => {
                    assert_eq!(preset.surface(), WindowSurface::Terminal);
                    assert!(preset.requires_process());
                    assert_eq!(preset.command_name(), Some("codex"));
                }
                WindowPreset::FileTree => {
                    assert_eq!(preset.surface(), WindowSurface::FileTree);
                    assert!(!preset.requires_process());
                }
                WindowPreset::Branches => {
                    assert_eq!(preset.surface(), WindowSurface::Branches);
                    assert!(!preset.requires_process());
                }
                WindowPreset::Settings => {
                    assert_eq!(preset.surface(), WindowSurface::Mock);
                    assert!(!preset.requires_process());
                    assert_eq!(preset.command_name(), None);
                }
                WindowPreset::Memo => {
                    assert_eq!(preset.surface(), WindowSurface::Memo);
                    assert!(!preset.requires_process());
                    assert_eq!(preset.command_name(), None);
                }
                WindowPreset::Profile => {
                    assert_eq!(preset.surface(), WindowSurface::Profile);
                    assert!(!preset.requires_process());
                    assert_eq!(preset.command_name(), None);
                }
                WindowPreset::Logs => {
                    assert_eq!(preset.surface(), WindowSurface::Logs);
                    assert!(!preset.requires_process());
                    assert_eq!(preset.command_name(), None);
                }
                WindowPreset::Board => {
                    assert_eq!(preset.surface(), WindowSurface::Board);
                    assert!(!preset.requires_process());
                    assert_eq!(preset.command_name(), None);
                }
                WindowPreset::Issue | WindowPreset::Spec | WindowPreset::Pr => {
                    assert_eq!(preset.surface(), WindowSurface::Knowledge);
                    assert!(!preset.requires_process());
                    assert_eq!(preset.command_name(), None);
                }
                WindowPreset::Agent => unreachable!("WindowPreset::ALL excludes Agent"),
            }
        }

        assert_eq!(WindowPreset::Agent.surface(), WindowSurface::Terminal);
        assert!(WindowPreset::Agent.requires_process());
        assert_eq!(WindowPreset::Agent.command_name(), None);
        assert_eq!(WindowPreset::Agent.default_size(), (720.0, 420.0));
    }

    #[test]
    fn resolve_codex_preset_includes_no_alt_screen_arg() {
        let shell = ShellProgram {
            command: "/bin/zsh".to_string(),
            args: vec![],
        };
        let result =
            resolve_launch_spec_with(WindowPreset::Codex, &shell, |command| command == "codex")
                .expect("codex preset should resolve");
        assert_eq!(result.command, "codex");
        assert!(
            result.args.iter().any(|arg| arg == "--no-alt-screen"),
            "Codex preset must launch with --no-alt-screen so inline scrollback survives \
             Plan-mode input waits (regression guard for Issue #2091)"
        );
    }

    #[test]
    fn resolve_codex_preset_launch_args_match_canonical_api() {
        // SPEC-1921 FR-064 / FR-065: preset path must produce exactly the
        // canonical launch args for the corresponding AgentId — no hard-coded
        // agent-specific flags allowed on the preset side.
        let shell = ShellProgram {
            command: "/bin/zsh".to_string(),
            args: vec![],
        };
        let result =
            resolve_launch_spec_with(WindowPreset::Codex, &shell, |command| command == "codex")
                .expect("codex preset should resolve");
        assert_eq!(
            result.args,
            gwt_agent::canonical_launch_args(&gwt_agent::AgentId::Codex),
            "preset Codex args must equal canonical_launch_args(&AgentId::Codex)"
        );
    }

    #[test]
    fn resolve_claude_preset_launch_args_match_canonical_api() {
        // SPEC-1921 FR-064 / FR-065: the preset layer must not hard-code
        // agent-specific defaults even for agents whose current canonical
        // list is empty — the equivalence must hold by construction.
        let shell = ShellProgram {
            command: "/bin/zsh".to_string(),
            args: vec![],
        };
        let result =
            resolve_launch_spec_with(WindowPreset::Claude, &shell, |command| command == "claude")
                .expect("claude preset should resolve");
        assert_eq!(
            result.args,
            gwt_agent::canonical_launch_args(&gwt_agent::AgentId::ClaudeCode),
            "preset Claude args must equal canonical_launch_args(&AgentId::ClaudeCode)"
        );
    }
}
