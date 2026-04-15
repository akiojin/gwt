use serde::{Deserialize, Serialize};
use std::path::Path;

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
    Mock,
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
            Self::Settings => "Placeholder settings surface",
            Self::Memo => "Placeholder notes surface",
            Self::Profile => "Placeholder profile surface",
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
            Self::Settings
            | Self::Memo
            | Self::Profile
            | Self::Logs
            | Self::Issue
            | Self::Spec
            | Self::Board
            | Self::Pr => WindowSurface::Mock,
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
                    args: Vec::new(),
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
                args: Vec::new(),
            });
        }
    }

    Err(PresetResolveError::ShellNotFound)
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
            if command_exists(command) {
                Ok(LaunchSpec {
                    title: preset.title().to_string(),
                    command: command.to_string(),
                    args: Vec::new(),
                })
            } else {
                Err(PresetResolveError::CommandNotFound {
                    command: command.to_string(),
                })
            }
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
}
