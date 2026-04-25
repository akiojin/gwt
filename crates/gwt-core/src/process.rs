//! Process execution helpers.

use std::process::{Command, Output};

use crate::error::{GwtError, Result};

/// Convert a completed process `Output` into a trimmed stdout `String`,
/// returning an error when the exit status is non-zero.
fn capture_output(cmd: &str, output: Output) -> Result<String> {
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(GwtError::Other(format!(
            "{cmd} exited with {}: {stderr}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Run a command and capture its stdout as a trimmed `String`.
///
/// Returns an error if the command fails to start or exits with a non-zero
/// status.
pub fn run_command(cmd: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(cmd)
        .args(args)
        .output()
        .map_err(GwtError::Io)?;
    capture_output(cmd, output)
}

/// Run a command with additional environment variables and capture its stdout.
pub fn run_command_with_env(cmd: &str, args: &[&str], env: &[(String, String)]) -> Result<String> {
    let mut command = Command::new(cmd);
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().map_err(GwtError::Io)?;
    capture_output(cmd, output)
}

/// Check whether a command exists on `$PATH`.
pub fn command_exists(cmd: &str) -> bool {
    which::which(cmd).is_ok()
}

/// Drop `GIT_*` env vars from `cmd` that override path-specific lookup.
///
/// When `git` runs from a hook context (e.g. husky pre-commit), it exports
/// `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, etc. Subprocesses inherit
/// them, and these env vars take precedence over `-C`/`current_dir`, so a
/// child `git` command nominally targeting a tempdir actually operates on
/// the calling repo. Apply this helper to a `Command` whose target is
/// determined by `current_dir` so the invocation stays hermetic.
pub fn scrub_git_env(cmd: &mut Command) -> &mut Command {
    cmd.env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_NAMESPACE")
        .env_remove("GIT_COMMON_DIR")
}

/// Get the version string of a command by running `<cmd> --version`.
pub fn get_command_version(cmd: &str) -> Result<String> {
    run_command(cmd, &["--version"])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_command(text: &str) -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), format!("echo {text}")],
            )
        } else {
            (
                "printf".to_string(),
                vec!["%s\\n".to_string(), text.to_string()],
            )
        }
    }

    fn failing_command() -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), "exit 1".to_string()],
            )
        } else {
            ("false".to_string(), Vec::new())
        }
    }

    fn env_echo_command(var_name: &str) -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), format!("echo %{var_name}%")],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), format!("echo ${var_name}")],
            )
        }
    }

    #[test]
    fn run_command_captures_stdout() {
        let (cmd, args) = echo_command("hello");
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command(&cmd, &arg_refs).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn run_command_trims_output() {
        let (cmd, args) = echo_command("  padded  ");
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command(&cmd, &arg_refs).unwrap();
        assert_eq!(result, "padded");
    }

    #[test]
    fn run_command_returns_error_on_failure() {
        let (cmd, args) = failing_command();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command(&cmd, &arg_refs);
        assert!(result.is_err());
    }

    #[test]
    fn run_command_returns_io_error_for_missing_binary() {
        let result = run_command("this_binary_does_not_exist_gwt_test", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn run_command_with_env_passes_env_vars() {
        let (cmd, args) = env_echo_command("GWT_TEST_VAR");
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command_with_env(
            &cmd,
            &arg_refs,
            &[("GWT_TEST_VAR".into(), "hello_env".into())],
        )
        .unwrap();
        assert_eq!(result, "hello_env");
    }

    #[test]
    fn run_command_with_env_returns_error_on_failure() {
        let (cmd, args) = failing_command();
        let arg_refs = args.iter().map(String::as_str).collect::<Vec<_>>();
        let result = run_command_with_env(&cmd, &arg_refs, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn command_exists_finds_echo() {
        assert!(command_exists("git"));
    }

    #[test]
    fn command_exists_returns_false_for_missing() {
        assert!(!command_exists("this_binary_does_not_exist_gwt_test"));
    }

    #[test]
    fn get_command_version_returns_version_string() {
        // `git --version` is universally available in dev environments
        let version = get_command_version("git").unwrap();
        assert!(version.contains("git version"));
    }
}
