//! Process execution helpers.

use std::{
    ffi::OsStr,
    process::{Command, Output},
};

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
    let output = hidden_command(cmd)
        .args(args)
        .output()
        .map_err(GwtError::Io)?;
    capture_output(cmd, output)
}

/// Run a command with additional environment variables and capture its stdout.
pub fn run_command_with_env(cmd: &str, args: &[&str], env: &[(String, String)]) -> Result<String> {
    let mut command = hidden_command(cmd);
    command.args(args);
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().map_err(GwtError::Io)?;
    capture_output(cmd, output)
}

// =====================================================================
// SPEC-1924 Phase C-git — synchronous git wrapper that emits
// `gwt.process.summary` start/end tracing and pushes captured stdout /
// stderr lines into the `ProcessConsoleHub`. Drop-in replacement for the
// common `hidden_command("git").args(...).current_dir(...).output()`
// idiom; preserves the std::process::Output return so callers do not
// need to change their downstream logic.
// =====================================================================

use std::sync::atomic::{AtomicU64, Ordering};

static GIT_SPAWN_COUNTER: AtomicU64 = AtomicU64::new(1);

fn next_git_spawn_id() -> u64 {
    GIT_SPAWN_COUNTER.fetch_add(1, Ordering::Relaxed)
}

/// Spawn `git` with `args` in `current_dir`, capture stdout / stderr,
/// emit Process facet summary tracing, and forward redacted lines to
/// the hub (kind = `Git`).
///
/// Returns the same `std::process::Output` as `Command::output()` so the
/// caller's existing `status.success()` / `stdout` / `stderr` handling
/// keeps working unchanged.
pub fn run_git_logged(
    args: &[&str],
    current_dir: Option<&std::path::Path>,
) -> std::io::Result<std::process::Output> {
    let spawn_id = next_git_spawn_id();
    let label = format!("git {}", args.join(" "));
    let started_at = std::time::Instant::now();

    tracing::info!(
        target: "gwt.process.summary",
        kind = "git",
        spawn_id = spawn_id,
        label = %label,
        phase = "start",
        "process start",
    );

    // SPEC-2809 (revised) — push the actual command line as a banner
    // into the hub so the Console window shows `$ git rev-parse ...`
    // instead of an opaque `spawn_id=N` marker. The frontend detects
    // the `$ ` prefix and styles it as an invocation header.
    push_command_banner_to_hub(
        crate::process_console::ProcessKind::Git,
        spawn_id,
        &label,
        current_dir,
    );

    let mut command = hidden_command("git");
    command.args(args);
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    let result = command.output();

    let (exit_code, success, stdout_lines, stderr_lines) = match &result {
        Ok(output) => {
            forward_git_lines(spawn_id, &output.stdout, &output.stderr);
            let stdout_lines = String::from_utf8_lossy(&output.stdout).lines().count() as u64;
            let stderr_lines = String::from_utf8_lossy(&output.stderr).lines().count() as u64;
            (
                output.status.code(),
                output.status.success(),
                stdout_lines,
                stderr_lines,
            )
        }
        Err(_) => (None, false, 0, 0),
    };

    let duration_ms = started_at.elapsed().as_millis() as u64;
    push_command_summary_to_hub(
        crate::process_console::ProcessKind::Git,
        spawn_id,
        exit_code,
        duration_ms,
    );

    tracing::info!(
        target: "gwt.process.summary",
        kind = "git",
        spawn_id = spawn_id,
        label = %label,
        phase = "end",
        exit_code = exit_code.map(|c| c as i64),
        duration_ms = duration_ms,
        stdout_lines = stdout_lines,
        stderr_lines = stderr_lines,
        success = success,
        "process end",
    );

    result
}

/// SPEC-2359 W-16 (FR-387): `run_git_logged` variant that pipes `stdin`
/// into the child — required by `git cat-file --batch-check`, which reads
/// its object list from stdin so one spawn resolves many blobs.
pub fn run_git_logged_with_stdin(
    args: &[&str],
    current_dir: Option<&std::path::Path>,
    stdin: &[u8],
) -> std::io::Result<std::process::Output> {
    use std::io::Write;

    let spawn_id = next_git_spawn_id();
    let label = format!("git {}", args.join(" "));
    let started_at = std::time::Instant::now();

    tracing::info!(
        target: "gwt.process.summary",
        kind = "git",
        spawn_id = spawn_id,
        label = %label,
        phase = "start",
        "process start",
    );
    push_command_banner_to_hub(
        crate::process_console::ProcessKind::Git,
        spawn_id,
        &label,
        current_dir,
    );

    let mut command = hidden_command("git");
    command.args(args);
    if let Some(dir) = current_dir {
        command.current_dir(dir);
    }
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let result = (|| {
        let mut child = command.spawn()?;
        if let Some(mut pipe) = child.stdin.take() {
            pipe.write_all(stdin)?;
        }
        child.wait_with_output()
    })();

    let (exit_code, success, stdout_lines, stderr_lines) = match &result {
        Ok(output) => {
            forward_git_lines(spawn_id, &output.stdout, &output.stderr);
            let stdout_lines = String::from_utf8_lossy(&output.stdout).lines().count() as u64;
            let stderr_lines = String::from_utf8_lossy(&output.stderr).lines().count() as u64;
            (
                output.status.code(),
                output.status.success(),
                stdout_lines,
                stderr_lines,
            )
        }
        Err(_) => (None, false, 0, 0),
    };

    let duration_ms = started_at.elapsed().as_millis() as u64;
    push_command_summary_to_hub(
        crate::process_console::ProcessKind::Git,
        spawn_id,
        exit_code,
        duration_ms,
    );
    tracing::info!(
        target: "gwt.process.summary",
        kind = "git",
        spawn_id = spawn_id,
        label = %label,
        phase = "end",
        exit_code = exit_code.map(|c| c as i64),
        duration_ms = duration_ms,
        stdout_lines = stdout_lines,
        stderr_lines = stderr_lines,
        success = success,
        "process end",
    );

    result
}

/// Shared helper: push a synthetic banner line (the command string
/// prefixed with `$ `) as the first line of a new spawn. Used by
/// `run_git_logged` and `spawn_logged` so the Console window can render
/// a per-invocation header without needing a separate metadata channel.
pub fn push_command_banner_to_hub(
    kind: crate::process_console::ProcessKind,
    spawn_id: u64,
    label: &str,
    current_dir: Option<&std::path::Path>,
) {
    let hub = crate::process_console::global();
    let banner = match current_dir {
        Some(dir) => format!("$ {} (cwd={})", label, dir.display()),
        None => format!("$ {label}"),
    };
    hub.push(crate::process_console::ProcessLine::new(
        kind,
        spawn_id,
        crate::process_console::ProcessStream::Stdout,
        banner,
    ));
}

/// Shared helper: push a synthetic summary line at spawn end with the
/// exit code + duration so the Console window shows a closing footer
/// per command.
pub fn push_command_summary_to_hub(
    kind: crate::process_console::ProcessKind,
    spawn_id: u64,
    exit_code: Option<i32>,
    duration_ms: u64,
) {
    let hub = crate::process_console::global();
    let exit = match exit_code {
        Some(code) => code.to_string(),
        None => "?".to_string(),
    };
    let footer = format!("→ exit={exit} ({duration_ms}ms)");
    hub.push(crate::process_console::ProcessLine::new(
        kind,
        spawn_id,
        crate::process_console::ProcessStream::Stdout,
        footer,
    ));
}

fn forward_git_lines(spawn_id: u64, stdout: &[u8], stderr: &[u8]) {
    let hub = crate::process_console::global();
    forward_bytes_to_hub(
        &hub,
        spawn_id,
        crate::process_console::ProcessStream::Stdout,
        stdout,
    );
    forward_bytes_to_hub(
        &hub,
        spawn_id,
        crate::process_console::ProcessStream::Stderr,
        stderr,
    );
}

fn forward_bytes_to_hub(
    hub: &crate::process_console::ProcessConsoleHub,
    spawn_id: u64,
    stream: crate::process_console::ProcessStream,
    bytes: &[u8],
) {
    if bytes.is_empty() {
        return;
    }
    let text = String::from_utf8_lossy(bytes);
    for piece in text.split(['\n', '\r']) {
        if piece.is_empty() {
            continue;
        }
        // Match the redact + ANSI strip discipline of the async
        // `spawn_logged` path so the hub never sees raw secrets or
        // ANSI sequences.
        let sanitized =
            crate::process_console::redact_line(&crate::process_console::strip_ansi(piece));
        hub.push(crate::process_console::ProcessLine::new(
            crate::process_console::ProcessKind::Git,
            spawn_id,
            stream,
            sanitized,
        ));
    }
}

/// Check whether a command exists on `$PATH`.
pub fn command_exists(cmd: &str) -> bool {
    which::which(cmd).is_ok()
}

/// Create a non-interactive command that does not create an extra console
/// window when spawned from the Windows GUI front door.
pub fn hidden_command<S: AsRef<OsStr>>(program: S) -> Command {
    let mut command = Command::new(program);
    configure_hidden_command(&mut command);
    command
}

/// Apply platform-specific non-interactive process flags.
pub fn configure_hidden_command(command: &mut Command) -> &mut Command {
    let flags = hidden_creation_flags();
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        command.creation_flags(flags);
    }
    #[cfg(not(windows))]
    {
        let _ = flags;
    }
    command
}

#[cfg(windows)]
fn hidden_creation_flags() -> u32 {
    0x08000000
}

#[cfg(not(windows))]
fn hidden_creation_flags() -> u32 {
    0
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

    #[test]
    fn hidden_creation_flags_match_platform() {
        if cfg!(windows) {
            assert_eq!(hidden_creation_flags(), 0x08000000);
        } else {
            assert_eq!(hidden_creation_flags(), 0);
        }
    }

    #[test]
    fn hidden_command_captures_stdout() {
        let (cmd, args) = echo_command("hidden hello");
        let output = hidden_command(&cmd)
            .args(args)
            .output()
            .expect("run hidden command");

        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "hidden hello"
        );
    }
}
