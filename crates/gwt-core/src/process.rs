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

/// Get the version string of a command by running `<cmd> --version`.
pub fn get_command_version(cmd: &str) -> Result<String> {
    run_command(cmd, &["--version"])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_command_captures_stdout() {
        let result = run_command("echo", &["hello"]).unwrap();
        assert_eq!(result, "hello");
    }

    #[test]
    fn run_command_trims_output() {
        let result = run_command("echo", &["  padded  "]).unwrap();
        assert_eq!(result, "padded");
    }

    #[test]
    fn run_command_returns_error_on_failure() {
        let result = run_command("false", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn run_command_returns_io_error_for_missing_binary() {
        let result = run_command("this_binary_does_not_exist_gwt_test", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn run_command_with_env_passes_env_vars() {
        let result = run_command_with_env(
            "sh",
            &["-c", "echo $GWT_TEST_VAR"],
            &[("GWT_TEST_VAR".into(), "hello_env".into())],
        )
        .unwrap();
        assert_eq!(result, "hello_env");
    }

    #[test]
    fn run_command_with_env_returns_error_on_failure() {
        let result = run_command_with_env("false", &[], &[]);
        assert!(result.is_err());
    }

    #[test]
    fn command_exists_finds_echo() {
        assert!(command_exists("echo"));
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
