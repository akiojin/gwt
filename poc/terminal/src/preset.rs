use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowPreset {
    Shell,
    Claude,
    Codex,
}

impl WindowPreset {
    pub fn title(self) -> &'static str {
        match self {
            Self::Shell => "Shell",
            Self::Claude => "Claude",
            Self::Codex => "Codex",
        }
    }

    pub fn command_name(self) -> Option<&'static str> {
        match self {
            Self::Shell => None,
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
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
