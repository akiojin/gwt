//! PTY management module
//!
//! Manages pseudo-terminal creation, I/O, resize, and cleanup.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use portable_pty::{native_pty_system, CommandBuilder, ExitStatus, MasterPty, PtySize};

use super::TerminalError;

/// Configuration for creating a new PTY.
pub struct PtyConfig {
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    pub env_vars: HashMap<String, String>,
    pub rows: u16,
    pub cols: u16,
    /// Optional shell override (e.g. "powershell", "cmd", "wsl").
    pub terminal_shell: Option<String>,
    /// Whether this is an interactive session (e.g. spawn_shell).
    /// When true on Windows, the command is not wrapped with PowerShell.
    pub interactive: bool,
    /// Whether to force UTF-8 terminal initialization on Windows launch.
    pub windows_force_utf8: bool,
}

fn escape_powershell_single_quoted(value: &str) -> String {
    value.replace('\'', "''")
}

fn build_windows_powershell_command_expression(
    command: &str,
    args: &[String],
    windows_force_utf8: bool,
) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(format!("'{}'", escape_powershell_single_quoted(command)));
    parts.extend(
        args.iter()
            .map(|arg| format!("'{}'", escape_powershell_single_quoted(arg))),
    );
    let invoke = format!("& {}", parts.join(" "));
    if windows_force_utf8 {
        format!(
            "$enc = [System.Text.UTF8Encoding]::new($false); [Console]::InputEncoding = $enc; [Console]::OutputEncoding = $enc; $OutputEncoding = $enc; chcp.com 65001 > $null; {invoke}"
        )
    } else {
        invoke
    }
}

fn resolve_windows_shell_with<F>(mut command_exists: F) -> String
where
    F: FnMut(&str) -> bool,
{
    if command_exists("pwsh") || command_exists("pwsh.exe") {
        "pwsh".to_string()
    } else {
        "powershell.exe".to_string()
    }
}

fn resolve_windows_shell() -> String {
    resolve_windows_shell_with(|command| which::which(command).is_ok())
}

fn escape_cmd_double_quoted(value: &str) -> String {
    value.replace('"', "\"\"")
}

fn quote_cmd_token_if_needed(value: &str) -> String {
    let needs_quotes = value.is_empty()
        || value.chars().any(|c| {
            c.is_whitespace()
                || matches!(c, '&' | '|' | '<' | '>' | '(' | ')' | '^' | '%' | '!' | '"')
        });

    if needs_quotes {
        format!("\"{}\"", escape_cmd_double_quoted(value))
    } else {
        value.to_string()
    }
}

fn build_cmd_command_expression(command: &str, args: &[String]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(quote_cmd_token_if_needed(command));
    parts.extend(args.iter().map(|arg| quote_cmd_token_if_needed(arg)));
    parts.join(" ")
}

fn strip_wrapping_quotes(value: &str) -> Option<&str> {
    if value.len() < 2 {
        return None;
    }

    let bytes = value.as_bytes();
    let first = bytes[0];
    let last = bytes[value.len() - 1];
    let wrapped = (first == b'"' && last == b'"') || (first == b'\'' && last == b'\'');
    if wrapped {
        Some(value[1..value.len() - 1].trim())
    } else {
        None
    }
}

fn strip_wrapping_escaped_quotes(value: &str) -> Option<&str> {
    if value.len() < 4 {
        return None;
    }

    let wrapped = (value.starts_with("\\\"") && value.ends_with("\\\""))
        || (value.starts_with("\\'") && value.ends_with("\\'"));
    if wrapped {
        Some(value[2..value.len() - 2].trim())
    } else {
        None
    }
}

fn strip_wrapping_quotes_recursive(value: &str) -> String {
    let mut current = value.trim();
    loop {
        if let Some(next) = strip_wrapping_quotes(current) {
            current = next;
            continue;
        }
        if let Some(next) = strip_wrapping_escaped_quotes(current) {
            current = next;
            continue;
        }
        break;
    }
    current.to_string()
}

fn has_windows_batch_extension(command: &str) -> bool {
    let normalized = strip_wrapping_quotes_recursive(command);
    Path::new(&normalized)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
        .unwrap_or(false)
}

fn is_windows_batch_command_for_platform<F>(command: &str, mut resolve_command_path: F) -> bool
where
    F: FnMut(&str) -> Option<PathBuf>,
{
    let normalized_command = strip_wrapping_quotes_recursive(command);

    if has_windows_batch_extension(&normalized_command) {
        return true;
    }

    if normalized_command.contains('\\') || normalized_command.contains('/') {
        return false;
    }

    resolve_command_path(&normalized_command)
        .map(|resolved| has_windows_batch_extension(resolved.to_string_lossy().as_ref()))
        .unwrap_or(false)
}

fn build_cmd_utf8_command_expression(command: &str, args: &[String]) -> String {
    let expression = build_cmd_command_expression(command, args);
    format!("chcp 65001 > nul && {expression}")
}

fn build_powershell_wrapped_args(expression: String) -> Vec<String> {
    vec![
        "-NoLogo".to_string(),
        "-NoProfile".to_string(),
        "-NonInteractive".to_string(),
        "-ExecutionPolicy".to_string(),
        "Bypass".to_string(),
        "-Command".to_string(),
        expression,
    ]
}

#[allow(clippy::too_many_arguments)]
fn resolve_spawn_command_for_platform_with<F, G>(
    command: &str,
    args: &[String],
    is_windows: bool,
    mut resolve_windows_shell: F,
    mut resolve_command_path: G,
    shell: Option<&str>,
    interactive: bool,
    windows_force_utf8: bool,
) -> (String, Vec<String>)
where
    F: FnMut() -> String,
    G: FnMut(&str) -> Option<PathBuf>,
{
    if is_windows {
        let normalized_command = strip_wrapping_quotes_recursive(command);

        if let Some(shell_id) = shell {
            match shell_id {
                "cmd" => {
                    let expression = if windows_force_utf8 {
                        build_cmd_utf8_command_expression(&normalized_command, args)
                    } else {
                        build_cmd_command_expression(&normalized_command, args)
                    };
                    let cmd_args = if windows_force_utf8 {
                        vec![
                            "/D".to_string(),
                            "/S".to_string(),
                            "/C".to_string(),
                            expression,
                        ]
                    } else {
                        vec!["/C".to_string(), expression]
                    };
                    return ("cmd.exe".to_string(), cmd_args);
                }
                // "wsl": command and args are already set by the caller (launch_with_wsl_pty_write
                // or resolve_shell_for_spawn), so pass through without wrapping.
                "wsl" => return (normalized_command.clone(), args.to_vec()),
                "powershell" => {
                    let shell = resolve_windows_shell();
                    let expression = build_windows_powershell_command_expression(
                        &normalized_command,
                        args,
                        windows_force_utf8,
                    );
                    return (shell, build_powershell_wrapped_args(expression));
                }
                _ => {}
            }
        }

        // Auto shell + batch command (`*.cmd`/`*.bat`) must run via `cmd.exe /C`
        // to keep Windows interactive PTY behavior stable across agents.
        if is_windows_batch_command_for_platform(&normalized_command, &mut resolve_command_path) {
            let expression = if windows_force_utf8 {
                build_cmd_utf8_command_expression(&normalized_command, args)
            } else {
                build_cmd_command_expression(&normalized_command, args)
            };
            let cmd_args = if windows_force_utf8 {
                vec![
                    "/D".to_string(),
                    "/S".to_string(),
                    "/C".to_string(),
                    expression,
                ]
            } else {
                vec!["/C".to_string(), expression]
            };
            return ("cmd.exe".to_string(), cmd_args);
        }

        // Interactive sessions (e.g. spawn_shell) must not be wrapped with
        // PowerShell -NonInteractive, as that breaks ConPTY I/O.
        if interactive && !windows_force_utf8 {
            return (normalized_command, args.to_vec());
        }

        if windows_force_utf8 {
            let expression = build_cmd_utf8_command_expression(&normalized_command, args);
            return (
                "cmd.exe".to_string(),
                vec![
                    "/D".to_string(),
                    "/S".to_string(),
                    "/C".to_string(),
                    expression,
                ],
            );
        }

        let shell = resolve_windows_shell();
        let expression =
            build_windows_powershell_command_expression(&normalized_command, args, false);
        return (shell, build_powershell_wrapped_args(expression));
    }

    (command.to_string(), args.to_vec())
}

fn resolve_spawn_command_for_platform<F>(
    command: &str,
    args: &[String],
    is_windows: bool,
    resolve_windows_shell: F,
    shell: Option<&str>,
    interactive: bool,
    windows_force_utf8: bool,
) -> (String, Vec<String>)
where
    F: FnMut() -> String,
{
    resolve_spawn_command_for_platform_with(
        command,
        args,
        is_windows,
        resolve_windows_shell,
        |name| {
            if cfg!(test) {
                None
            } else {
                which::which(name).ok()
            }
        },
        shell,
        interactive,
        windows_force_utf8,
    )
}

fn resolve_spawn_command(
    command: &str,
    args: &[String],
    shell: Option<&str>,
    interactive: bool,
    windows_force_utf8: bool,
) -> (String, Vec<String>) {
    resolve_spawn_command_for_platform(
        command,
        args,
        cfg!(windows),
        resolve_windows_shell,
        shell,
        interactive,
        windows_force_utf8,
    )
}

/// Handle to a PTY instance with its child process.
pub struct PtyHandle {
    master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
}

impl PtyHandle {
    /// Create a new PTY, spawn the given command, and return a handle.
    pub fn new(config: PtyConfig) -> Result<Self, TerminalError> {
        if !config.working_dir.exists() {
            return Err(TerminalError::PtyCreationFailed {
                reason: format!(
                    "Working directory does not exist: {}",
                    config.working_dir.display()
                ),
            });
        }

        let pty_system = native_pty_system();

        let size = PtySize {
            rows: config.rows,
            cols: config.cols,
            pixel_width: 0,
            pixel_height: 0,
        };

        let pair = pty_system
            .openpty(size)
            .map_err(|e| TerminalError::PtyCreationFailed {
                reason: e.to_string(),
            })?;

        let (spawn_command, spawn_args) = resolve_spawn_command(
            &config.command,
            &config.args,
            config.terminal_shell.as_deref(),
            config.interactive,
            config.windows_force_utf8,
        );
        let launch_mode = if cfg!(windows) {
            if spawn_command.eq_ignore_ascii_case("cmd.exe")
                && spawn_args
                    .first()
                    .is_some_and(|arg| arg.eq_ignore_ascii_case("/c"))
            {
                "cmd-wrapper"
            } else if spawn_args.iter().any(|arg| arg == "-Command") {
                "powershell-wrapper"
            } else {
                "pass-through"
            }
        } else {
            "native"
        };

        tracing::debug!(
            command = %config.command,
            args = ?config.args,
            resolved_command = %spawn_command,
            resolved_args = ?spawn_args,
            interactive = config.interactive,
            terminal_shell = ?config.terminal_shell,
            launch_mode,
            "Resolved PTY spawn command"
        );

        let mut cmd = CommandBuilder::new(&spawn_command);
        for arg in &spawn_args {
            cmd.arg(arg);
        }
        cmd.cwd(&config.working_dir);

        // Default to a color-capable terminal environment.
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");

        // Set user-provided environment variables
        for (key, value) in &config.env_vars {
            cmd.env(key, value);
        }

        let child =
            pair.slave
                .spawn_command(cmd)
                .map_err(|e| TerminalError::PtyCreationFailed {
                    reason: e.to_string(),
                })?;

        // Drop slave after spawning (required by portable-pty)
        drop(pair.slave);

        Ok(Self {
            master: pair.master,
            child,
        })
    }

    /// Get a cloneable reader for PTY output.
    pub fn take_reader(&self) -> Result<Box<dyn Read + Send>, TerminalError> {
        self.master
            .try_clone_reader()
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Take the single writer for PTY input.
    pub fn take_writer(&self) -> Result<Box<dyn Write + Send>, TerminalError> {
        self.master
            .take_writer()
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Resize the PTY to the given dimensions.
    pub fn resize(&self, rows: u16, cols: u16) -> Result<(), TerminalError> {
        self.master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Non-blocking check if the child process has exited.
    pub fn try_wait(&mut self) -> Result<Option<ExitStatus>, TerminalError> {
        self.child
            .try_wait()
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Kill the child process.
    pub fn kill(&mut self) -> Result<(), TerminalError> {
        self.child.kill().map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn escape_powershell_single_quoted_duplicates_single_quotes() {
        assert_eq!(
            escape_powershell_single_quoted("C:\\Tools\\it's\\npx.cmd"),
            "C:\\Tools\\it''s\\npx.cmd"
        );
    }

    #[test]
    fn build_windows_powershell_command_expression_quotes_command_and_args() {
        let args = vec!["--yes".to_string(), "@openai/codex@latest".to_string()];
        let expr = build_windows_powershell_command_expression("C:\\Tools\\npx.cmd", &args, false);
        assert_eq!(
            expr,
            "& 'C:\\Tools\\npx.cmd' '--yes' '@openai/codex@latest'"
        );
    }

    #[test]
    fn build_windows_powershell_command_expression_utf8_wraps_preamble() {
        let args = vec!["--version".to_string()];
        let expr = build_windows_powershell_command_expression("codex", &args, true);
        assert!(expr.contains("[System.Text.UTF8Encoding]::new($false)"));
        assert!(expr.contains("chcp.com 65001 > $null"));
        assert!(expr.ends_with("& 'codex' '--version'"));
    }

    #[test]
    fn resolve_windows_shell_prefers_pwsh_when_available() {
        let shell = resolve_windows_shell_with(|name| name == "pwsh");
        assert_eq!(shell, "pwsh");
    }

    #[test]
    fn resolve_windows_shell_falls_back_to_windows_powershell() {
        let shell = resolve_windows_shell_with(|_| false);
        assert_eq!(shell, "powershell.exe");
    }

    #[test]
    fn resolve_spawn_command_wraps_command_for_windows_platform() {
        let args = vec!["--yes".to_string(), "@openai/codex@latest".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "C:\\Tools\\npx.cmd",
            &args,
            true,
            || "pwsh".to_string(),
            None,
            false,
            false,
        );

        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/C".to_string(),
                "C:\\Tools\\npx.cmd --yes @openai/codex@latest".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_windows_platform_wraps_non_batch_command() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "codex",
            &args,
            true,
            || "powershell.exe".to_string(),
            None,
            false,
            false,
        );
        assert_eq!(program, "powershell.exe");
        assert_eq!(
            resolved_args,
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                "& 'codex' '--version'".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_non_windows_keeps_original_command() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "codex",
            &args,
            false,
            || "pwsh".to_string(),
            None,
            false,
            false,
        );
        assert_eq!(program, "codex");
        assert_eq!(resolved_args, args);
    }

    #[test]
    fn resolve_spawn_command_cmd_shell_uses_cmd_exe() {
        let args = vec!["--yes".to_string(), "@openai/codex@latest".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "npx.cmd",
            &args,
            true,
            || "pwsh".to_string(),
            Some("cmd"),
            false,
            false,
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/C".to_string(),
                "npx.cmd --yes @openai/codex@latest".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_cmd_shell_quotes_spaces_and_metacharacters() {
        let args = vec![
            "--cwd".to_string(),
            "C:\\Users\\Test User\\repo".to_string(),
            "a&b".to_string(),
        ];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "C:\\Program Files\\nodejs\\npx.cmd",
            &args,
            true,
            || "pwsh".to_string(),
            Some("cmd"),
            false,
            false,
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/C".to_string(),
                "\"C:\\Program Files\\nodejs\\npx.cmd\" --cwd \"C:\\Users\\Test User\\repo\" \"a&b\""
                    .to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_windows_force_utf8_wraps_with_cmd() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "codex",
            &args,
            true,
            || "pwsh".to_string(),
            None,
            true,
            true,
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/D".to_string(),
                "/S".to_string(),
                "/C".to_string(),
                "chcp 65001 > nul && codex --version".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_windows_force_utf8_cmd_shell_uses_cmd_wrapper() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "codex",
            &args,
            true,
            || "pwsh".to_string(),
            Some("cmd"),
            false,
            true,
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/D".to_string(),
                "/S".to_string(),
                "/C".to_string(),
                "chcp 65001 > nul && codex --version".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_windows_force_utf8_powershell_shell_keeps_powershell_wrapper() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "codex",
            &args,
            true,
            || "pwsh".to_string(),
            Some("powershell"),
            false,
            true,
        );
        assert_eq!(program, "pwsh");
        assert_eq!(
            &resolved_args[0..6],
            &[
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
            ]
        );
        assert!(resolved_args[6].contains("[System.Text.UTF8Encoding]::new($false)"));
        assert!(resolved_args[6].contains("chcp.com 65001 > $null"));
        assert!(resolved_args[6].contains("& 'codex' '--version'"));
    }

    #[test]
    fn resolve_spawn_command_windows_force_utf8_wsl_passthrough() {
        let args = vec!["-e".to_string(), "echo hello".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "wsl.exe",
            &args,
            true,
            || "pwsh".to_string(),
            Some("wsl"),
            true,
            true,
        );
        assert_eq!(program, "wsl.exe");
        assert_eq!(resolved_args, args);
    }

    #[test]
    fn build_cmd_command_expression_escapes_embedded_quotes() {
        let expression =
            build_cmd_command_expression("tool.cmd", &[r#"arg "quoted" value"#.to_string()]);
        assert_eq!(expression, r#"tool.cmd "arg ""quoted"" value""#);
    }

    #[test]
    fn strip_wrapping_quotes_recursive_unwraps_nested_quotes() {
        assert_eq!(
            strip_wrapping_quotes_recursive("'\"C:\\Program Files\\nodejs\\npx.cmd\"'"),
            "C:\\Program Files\\nodejs\\npx.cmd"
        );
        assert_eq!(
            strip_wrapping_quotes_recursive("\"C:\\Tools\\npx.cmd\""),
            "C:\\Tools\\npx.cmd"
        );
        assert_eq!(
            strip_wrapping_quotes_recursive(r#"'\"C:\Program Files\nodejs\npx.cmd\"'"#),
            r#"C:\Program Files\nodejs\npx.cmd"#
        );
        assert_eq!(
            strip_wrapping_quotes_recursive(r#"\"C:\Tools\npx.cmd\""#),
            r#"C:\Tools\npx.cmd"#
        );
    }

    #[test]
    fn is_windows_batch_command_for_platform_detects_cmd_extension() {
        assert!(is_windows_batch_command_for_platform(
            "C:\\Users\\user\\AppData\\Roaming\\npm\\npx.CMD",
            |_| None
        ));
    }

    #[test]
    fn is_windows_batch_command_for_platform_detects_wrapped_cmd_extension() {
        assert!(is_windows_batch_command_for_platform(
            "'\"C:\\Program Files\\nodejs\\npx.cmd\"'",
            |_| None
        ));
    }

    #[test]
    fn is_windows_batch_command_for_platform_detects_escaped_wrapped_cmd_extension() {
        assert!(is_windows_batch_command_for_platform(
            r#"'\"C:\Program Files\nodejs\npx.cmd\"'"#,
            |_| None
        ));
    }

    #[test]
    fn is_windows_batch_command_for_platform_detects_resolved_cmd_for_bare_command() {
        assert!(is_windows_batch_command_for_platform("claude", |name| {
            assert_eq!(name, "claude");
            Some(PathBuf::from(
                "C:\\Users\\user\\AppData\\Roaming\\npm\\claude.cmd",
            ))
        }));
    }

    #[test]
    fn is_windows_batch_command_for_platform_rejects_non_batch_extension() {
        assert!(!is_windows_batch_command_for_platform(
            "C:\\Tools\\pwsh.exe",
            |_| None
        ));
    }

    #[test]
    fn resolve_spawn_command_windows_normalizes_wrapped_batch_path() {
        let args = vec!["--yes".to_string(), "@openai/codex@latest".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "'\"C:\\Program Files\\nodejs\\npx.cmd\"'",
            &args,
            true,
            || "pwsh".to_string(),
            None,
            false,
            false,
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/C".to_string(),
                "\"C:\\Program Files\\nodejs\\npx.cmd\" --yes @openai/codex@latest".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_windows_normalizes_escaped_wrapped_batch_path() {
        let args = vec!["--yes".to_string(), "@openai/codex@latest".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            r#"'\"C:\Program Files\nodejs\npx.cmd\"'"#,
            &args,
            true,
            || "pwsh".to_string(),
            None,
            false,
            false,
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/C".to_string(),
                "\"C:\\Program Files\\nodejs\\npx.cmd\" --yes @openai/codex@latest".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_powershell_shell_uses_default_powershell() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "codex",
            &args,
            true,
            || "pwsh".to_string(),
            Some("powershell"),
            false,
            false,
        );
        assert_eq!(program, "pwsh");
        assert_eq!(
            resolved_args,
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                "& 'codex' '--version'".to_string(),
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_powershell_shell_interactive_keeps_wrapper() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "codex",
            &args,
            true,
            || "pwsh".to_string(),
            Some("powershell"),
            true,
            false,
        );
        assert_eq!(program, "pwsh");
        assert_eq!(
            resolved_args,
            vec![
                "-NoLogo".to_string(),
                "-NoProfile".to_string(),
                "-NonInteractive".to_string(),
                "-ExecutionPolicy".to_string(),
                "Bypass".to_string(),
                "-Command".to_string(),
                "& 'codex' '--version'".to_string(),
            ]
        );
    }

    /// Helper: read from PTY reader in a separate thread with timeout.
    fn read_with_timeout(
        mut reader: Box<dyn Read + Send>,
        timeout: Duration,
    ) -> Result<String, String> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 4096];
            let mut output = Vec::new();
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        output.extend_from_slice(&buf[..n]);
                        let _ = tx.send(Ok(String::from_utf8_lossy(&output).to_string()));
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        break;
                    }
                }
            }
        });

        // Collect output until timeout
        let mut last_output = String::new();
        let deadline = std::time::Instant::now() + timeout;
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(s)) => last_output = s,
                Ok(Err(e)) => return Err(e),
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if !last_output.is_empty() {
                        return Ok(last_output);
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        if last_output.is_empty() {
            Err("Timed out with no output".to_string())
        } else {
            Ok(last_output)
        }
    }

    #[test]
    fn test_pty_creation_and_echo() {
        let config = PtyConfig {
            command: "/bin/echo".to_string(),
            args: vec!["hello".to_string()],
            working_dir: std::env::temp_dir(),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
            terminal_shell: None,
            interactive: false,
            windows_force_utf8: false,
        };

        let handle = PtyHandle::new(config).expect("Failed to create PTY");
        let reader = handle.take_reader().expect("Failed to get reader");

        let output =
            read_with_timeout(reader, Duration::from_secs(5)).expect("Failed to read PTY output");

        assert!(
            output.contains("hello"),
            "Expected 'hello' in output, got: {output}"
        );
    }

    #[test]
    fn test_env_vars_set() {
        let mut env_vars = HashMap::new();
        env_vars.insert("GWT_PANE_ID".to_string(), "pane-42".to_string());
        env_vars.insert("GWT_BRANCH".to_string(), "feature/test".to_string());
        env_vars.insert("GWT_SESSION_ID".to_string(), "sess-001".to_string());

        let config = PtyConfig {
            command: "/usr/bin/env".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            env_vars,
            rows: 24,
            cols: 80,
            terminal_shell: None,
            interactive: false,
            windows_force_utf8: false,
        };

        let handle = PtyHandle::new(config).expect("Failed to create PTY");
        let reader = handle.take_reader().expect("Failed to get reader");

        let output =
            read_with_timeout(reader, Duration::from_secs(5)).expect("Failed to read PTY output");

        assert!(
            output.contains("GWT_PANE_ID=pane-42"),
            "Expected GWT_PANE_ID in output, got: {output}"
        );
        assert!(
            output.contains("GWT_BRANCH=feature/test"),
            "Expected GWT_BRANCH in output, got: {output}"
        );
        assert!(
            output.contains("GWT_SESSION_ID=sess-001"),
            "Expected GWT_SESSION_ID in output, got: {output}"
        );
        assert!(
            output.contains("TERM=xterm-256color"),
            "Expected TERM=xterm-256color in output, got: {output}"
        );
        assert!(
            output.contains("COLORTERM=truecolor"),
            "Expected COLORTERM=truecolor in output, got: {output}"
        );
    }

    #[test]
    fn test_resize() {
        let config = PtyConfig {
            command: "/bin/sleep".to_string(),
            args: vec!["1".to_string()],
            working_dir: std::env::temp_dir(),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
            terminal_shell: None,
            interactive: false,
            windows_force_utf8: false,
        };

        let handle = PtyHandle::new(config).expect("Failed to create PTY");
        let result = handle.resize(48, 120);
        assert!(result.is_ok(), "Resize should succeed: {result:?}");
    }

    #[test]
    fn test_process_exit_detection() {
        let config = PtyConfig {
            command: "/usr/bin/true".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
            terminal_shell: None,
            interactive: false,
            windows_force_utf8: false,
        };

        let mut handle = PtyHandle::new(config).expect("Failed to create PTY");

        // Wait for process to exit
        let mut exited = false;
        for _ in 0..50 {
            if let Ok(Some(_status)) = handle.try_wait() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        assert!(exited, "Process should have exited");
    }

    #[test]
    fn test_invalid_command_error() {
        let config = PtyConfig {
            command: "/nonexistent/command/that/does/not/exist".to_string(),
            args: vec![],
            working_dir: std::env::temp_dir(),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
            terminal_shell: None,
            interactive: false,
            windows_force_utf8: false,
        };

        let result = PtyHandle::new(config);
        assert!(
            result.is_err(),
            "Creating PTY with invalid command should fail"
        );

        if let Err(TerminalError::PtyCreationFailed { reason }) = result {
            assert!(
                !reason.is_empty(),
                "Error reason should not be empty: {reason}"
            );
        } else {
            panic!("Expected TerminalError::PtyCreationFailed");
        }
    }

    #[test]
    fn resolve_spawn_command_interactive_windows_passes_through() {
        let args = vec!["--version".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "pwsh",
            &args,
            true,
            || "pwsh".to_string(),
            None,
            true,
            false,
        );
        assert_eq!(program, "pwsh");
        assert_eq!(resolved_args, vec!["--version".to_string()]);
    }

    #[test]
    fn resolve_spawn_command_interactive_non_windows_unchanged() {
        let args = vec!["-l".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "/bin/zsh",
            &args,
            false,
            || "pwsh".to_string(),
            None,
            true,
            false,
        );
        assert_eq!(program, "/bin/zsh");
        assert_eq!(resolved_args, vec!["-l".to_string()]);
    }

    #[test]
    fn resolve_spawn_command_windows_agent_host_os_interactive() {
        let args = vec!["--dangerously-skip-permissions".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "claude",
            &args,
            true,
            || "pwsh".to_string(),
            None,
            true,
            false,
        );
        assert_eq!(program, "claude");
        assert_eq!(
            resolved_args,
            vec!["--dangerously-skip-permissions".to_string()]
        );
    }

    #[test]
    fn resolve_spawn_command_windows_agent_host_os_interactive_resolved_cmd_uses_cmd_exe() {
        let args = vec!["--dangerously-skip-permissions".to_string()];
        let (program, resolved_args) = resolve_spawn_command_for_platform_with(
            "claude",
            &args,
            true,
            || "pwsh".to_string(),
            |name| {
                if name == "claude" {
                    Some(PathBuf::from(
                        "C:\\Users\\user\\AppData\\Roaming\\npm\\claude.cmd",
                    ))
                } else {
                    None
                }
            },
            None,
            true,
            false,
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/C".to_string(),
                "claude --dangerously-skip-permissions".to_string()
            ]
        );
    }

    #[test]
    fn resolve_spawn_command_windows_agent_bunx_interactive() {
        let args = vec![
            "--yes".to_string(),
            "@anthropic/claude-code@latest".to_string(),
        ];
        let (program, resolved_args) = resolve_spawn_command_for_platform(
            "C:\\Users\\user\\.bun\\bin\\bunx.cmd",
            &args,
            true,
            || "pwsh".to_string(),
            None,
            true,
            false,
        );
        assert_eq!(program, "cmd.exe");
        assert_eq!(
            resolved_args,
            vec![
                "/C".to_string(),
                "C:\\Users\\user\\.bun\\bin\\bunx.cmd --yes @anthropic/claude-code@latest"
                    .to_string(),
            ]
        );
    }

    #[test]
    fn test_pty_creation_fails_with_nonexistent_working_dir() {
        let config = PtyConfig {
            command: "/bin/echo".to_string(),
            args: vec!["hello".to_string()],
            working_dir: PathBuf::from("/nonexistent/path/that/does/not/exist"),
            env_vars: HashMap::new(),
            rows: 24,
            cols: 80,
            terminal_shell: None,
            interactive: false,
            windows_force_utf8: false,
        };

        let result = PtyHandle::new(config);
        assert!(
            result.is_err(),
            "Creating PTY with nonexistent working dir should fail"
        );

        if let Err(TerminalError::PtyCreationFailed { reason }) = result {
            assert!(
                reason.contains("Working directory does not exist"),
                "Error should mention working directory: {reason}"
            );
        } else {
            panic!("Expected TerminalError::PtyCreationFailed");
        }
    }
}
