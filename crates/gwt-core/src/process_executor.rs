//! Process execution seam for testable external-command flows (SPEC-3014).
//!
//! [`ProcessExecutor`] abstracts the three `std::process::Command` idioms
//! used by `gwt-core::update` (FR-001):
//!
//! - [`ExecutionMode::Spawn`] — `Command::spawn()`: start the process and
//!   return immediately without waiting for it to exit (update-helper
//!   relaunch, post-update app restart).
//! - [`ExecutionMode::Status`] — `Command::status()`: wait for the process to
//!   exit with inherited stdio (osascript / hdiutil / powershell installer
//!   flows).
//! - [`ExecutionMode::Capture`] — `Command::output()`: wait for the process
//!   to exit and capture stdout/stderr (`kill -0` process probe).
//!
//! [`SystemProcessExecutor`] is the production implementation backed by
//! `std::process::Command` (reusing [`crate::process::hidden_command`] for
//! Windows window hiding when [`ProcessRequest::hide_window`] is set).
//! [`MockProcessExecutor`] records every request and returns scripted outputs
//! so platform-specific flows can be unit-tested on any OS without spawning
//! real processes (FR-003). The mock is a plain `pub` type (no `cfg(test)`
//! gating) so downstream crates' tests can inject it.

use std::collections::{HashMap, VecDeque};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How a [`ProcessRequest`] should be executed. Mirrors the
/// `std::process::Command` idioms documented in the module header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionMode {
    /// `Command::spawn()` — fire-and-forget. The returned
    /// [`ProcessOutput`] only signals that the spawn succeeded.
    Spawn,
    /// `Command::status()` — wait for exit, stdio inherited. The returned
    /// [`ProcessOutput`] carries the exit status but no stdout/stderr.
    Status,
    /// `Command::output()` — wait for exit and capture stdout/stderr.
    Capture,
}

/// A description of one external process invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessRequest {
    /// Program path or name (resolved via `PATH` like `Command::new`).
    pub program: OsString,
    /// Arguments passed verbatim to the program.
    pub args: Vec<OsString>,
    /// Optional working directory (`Command::current_dir`).
    pub cwd: Option<PathBuf>,
    /// Additional environment variables (`Command::env`).
    pub env: Vec<(String, String)>,
    /// Execution idiom; defaults to [`ExecutionMode::Status`].
    pub mode: ExecutionMode,
    /// Apply the Windows window-hiding creation flags
    /// ([`crate::process::hidden_command`]). No-op on other platforms.
    pub hide_window: bool,
}

impl ProcessRequest {
    /// Start building a request for `program`.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_os_string(),
            args: Vec::new(),
            cwd: None,
            env: Vec::new(),
            mode: ExecutionMode::Status,
            hide_window: false,
        }
    }

    /// Append one argument.
    #[must_use]
    pub fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        self.args.push(arg.as_ref().to_os_string());
        self
    }

    /// Append multiple arguments.
    #[must_use]
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_os_string()));
        self
    }

    /// Set the working directory.
    #[must_use]
    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    /// Add an environment variable.
    #[must_use]
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    /// Set the execution mode.
    #[must_use]
    pub fn mode(mut self, mode: ExecutionMode) -> Self {
        self.mode = mode;
        self
    }

    /// Request Windows window hiding (see [`ProcessRequest::hide_window`]).
    #[must_use]
    pub fn hide_window(mut self, hide: bool) -> Self {
        self.hide_window = hide;
        self
    }

    /// The program's file name (lossy), e.g. `"hdiutil"` for
    /// `/usr/bin/hdiutil`. Used by [`MockProcessExecutor`] program matching.
    pub fn program_name(&self) -> String {
        Path::new(&self.program)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.program.to_string_lossy().into_owned())
    }

    /// Arguments converted to lossy `String`s for ergonomic assertions.
    pub fn args_lossy(&self) -> Vec<String> {
        self.args
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }
}

/// Result of a completed (or spawned) process invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessOutput {
    /// Whether the invocation succeeded. For [`ExecutionMode::Spawn`] this
    /// means the spawn itself succeeded; otherwise the exit status was zero.
    pub success: bool,
    /// Exit code when the process exited with one (`None` for spawn-only
    /// invocations or signal-terminated processes).
    pub code: Option<i32>,
    /// Human-readable status used in error messages. The system executor
    /// fills this from `std::process::ExitStatus`'s `Display` (e.g.
    /// `"exit status: 1"`) so messages stay identical to the pre-seam code.
    pub status_display: String,
    /// Captured stdout (lossy UTF-8). Empty unless [`ExecutionMode::Capture`].
    pub stdout: String,
    /// Captured stderr (lossy UTF-8). Empty unless [`ExecutionMode::Capture`].
    pub stderr: String,
}

impl ProcessOutput {
    /// A zero exit status.
    pub fn succeeded() -> Self {
        Self {
            success: true,
            code: Some(0),
            status_display: "exit status: 0".to_string(),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    /// A non-zero exit status with the given code.
    pub fn failed(code: i32) -> Self {
        Self {
            success: false,
            code: Some(code),
            status_display: format!("exit status: {code}"),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    /// A successful fire-and-forget spawn (no exit status available).
    pub fn spawned() -> Self {
        Self {
            success: true,
            code: None,
            status_display: "spawned".to_string(),
            stdout: String::new(),
            stderr: String::new(),
        }
    }

    /// Attach stdout text (builder-style, for mock scripting).
    #[must_use]
    pub fn with_stdout(mut self, stdout: impl Into<String>) -> Self {
        self.stdout = stdout.into();
        self
    }

    /// Attach stderr text (builder-style, for mock scripting).
    #[must_use]
    pub fn with_stderr(mut self, stderr: impl Into<String>) -> Self {
        self.stderr = stderr.into();
        self
    }
}

/// Abstraction over external process execution.
///
/// `Err(String)` corresponds to a failure to start the process (the
/// `io::Error` from `Command::spawn/status/output`, stringified), while a
/// non-zero exit is reported through [`ProcessOutput::success`].
pub trait ProcessExecutor: Send + Sync {
    /// Execute `request` according to its [`ExecutionMode`].
    fn run(&self, request: ProcessRequest) -> Result<ProcessOutput, String>;
}

/// Production [`ProcessExecutor`] backed by `std::process::Command`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemProcessExecutor;

impl ProcessExecutor for SystemProcessExecutor {
    fn run(&self, request: ProcessRequest) -> Result<ProcessOutput, String> {
        let mut command = if request.hide_window {
            crate::process::hidden_command(&request.program)
        } else {
            std::process::Command::new(&request.program)
        };
        command.args(&request.args);
        if let Some(cwd) = &request.cwd {
            command.current_dir(cwd);
        }
        for (key, value) in &request.env {
            command.env(key, value);
        }

        match request.mode {
            ExecutionMode::Spawn => {
                command.spawn().map_err(|e| e.to_string())?;
                Ok(ProcessOutput::spawned())
            }
            ExecutionMode::Status => {
                let status = command.status().map_err(|e| e.to_string())?;
                Ok(output_from_status(status, String::new(), String::new()))
            }
            ExecutionMode::Capture => {
                let output = command.output().map_err(|e| e.to_string())?;
                Ok(output_from_status(
                    output.status,
                    String::from_utf8_lossy(&output.stdout).into_owned(),
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                ))
            }
        }
    }
}

fn output_from_status(
    status: std::process::ExitStatus,
    stdout: String,
    stderr: String,
) -> ProcessOutput {
    ProcessOutput {
        success: status.success(),
        code: status.code(),
        status_display: status.to_string(),
        stdout,
        stderr,
    }
}

/// Recording [`ProcessExecutor`] for tests.
///
/// Responses are resolved in this order:
/// 1. a program-scripted response queued via
///    [`MockProcessExecutor::push_program_response`] /
///    [`MockProcessExecutor::push_program_error`] whose key matches the
///    request's full program string or its file name,
/// 2. the global in-order queue
///    ([`MockProcessExecutor::push_response`] / `push_error`),
/// 3. the default response ([`MockProcessExecutor::set_default_response`]),
/// 4. otherwise `Err` describing the unexpected invocation.
#[derive(Debug, Default)]
pub struct MockProcessExecutor {
    requests: Mutex<Vec<ProcessRequest>>,
    queue: Mutex<VecDeque<Result<ProcessOutput, String>>>,
    by_program: Mutex<HashMap<String, VecDeque<Result<ProcessOutput, String>>>>,
    default_response: Mutex<Option<Result<ProcessOutput, String>>>,
}

impl MockProcessExecutor {
    /// Create an empty mock (every invocation is unexpected until scripted).
    pub fn new() -> Self {
        Self::default()
    }

    /// Queue an in-order response.
    pub fn push_response(&self, output: ProcessOutput) {
        self.queue.lock().unwrap().push_back(Ok(output));
    }

    /// Queue an in-order spawn/start failure.
    pub fn push_error(&self, message: impl Into<String>) {
        self.queue.lock().unwrap().push_back(Err(message.into()));
    }

    /// Queue a response for invocations of `program` (full string or file
    /// name match).
    pub fn push_program_response(&self, program: impl Into<String>, output: ProcessOutput) {
        self.by_program
            .lock()
            .unwrap()
            .entry(program.into())
            .or_default()
            .push_back(Ok(output));
    }

    /// Queue a spawn/start failure for invocations of `program`.
    pub fn push_program_error(&self, program: impl Into<String>, message: impl Into<String>) {
        self.by_program
            .lock()
            .unwrap()
            .entry(program.into())
            .or_default()
            .push_back(Err(message.into()));
    }

    /// Fallback response used when no scripted response matches.
    pub fn set_default_response(&self, output: ProcessOutput) {
        *self.default_response.lock().unwrap() = Some(Ok(output));
    }

    /// All recorded requests, in invocation order.
    pub fn requests(&self) -> Vec<ProcessRequest> {
        self.requests.lock().unwrap().clone()
    }

    /// Number of recorded requests.
    pub fn request_count(&self) -> usize {
        self.requests.lock().unwrap().len()
    }

    /// Recorded requests whose program matches `program` (full string or
    /// file name).
    pub fn requests_for(&self, program: &str) -> Vec<ProcessRequest> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| program_matches(program, request))
            .cloned()
            .collect()
    }
}

fn program_matches(key: &str, request: &ProcessRequest) -> bool {
    request.program.to_string_lossy() == key || request.program_name() == key
}

impl ProcessExecutor for MockProcessExecutor {
    fn run(&self, request: ProcessRequest) -> Result<ProcessOutput, String> {
        self.requests.lock().unwrap().push(request.clone());

        if let Some(response) = self.pop_program_response(&request) {
            return response;
        }
        if let Some(response) = self.queue.lock().unwrap().pop_front() {
            return response;
        }
        if let Some(default) = self.default_response.lock().unwrap().clone() {
            return default;
        }
        Err(format!(
            "MockProcessExecutor: unexpected process invocation: {} {:?}",
            request.program.to_string_lossy(),
            request.args_lossy()
        ))
    }
}

impl MockProcessExecutor {
    fn pop_program_response(
        &self,
        request: &ProcessRequest,
    ) -> Option<Result<ProcessOutput, String>> {
        let mut by_program = self.by_program.lock().unwrap();
        let key = by_program
            .iter()
            .filter(|(key, responses)| !responses.is_empty() && program_matches(key, request))
            .map(|(key, _)| key.clone())
            .next()?;
        by_program.get_mut(&key).and_then(VecDeque::pop_front)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── MockProcessExecutor contract (SPEC-3014 T-001) ─────────────────

    #[test]
    fn mock_records_requests_in_invocation_order() {
        let mock = MockProcessExecutor::new();
        mock.set_default_response(ProcessOutput::succeeded());

        mock.run(ProcessRequest::new("first").arg("a")).unwrap();
        mock.run(ProcessRequest::new("second").args(["b", "c"]))
            .unwrap();

        let requests = mock.requests();
        assert_eq!(mock.request_count(), 2);
        assert_eq!(requests[0].program_name(), "first");
        assert_eq!(requests[0].args_lossy(), vec!["a"]);
        assert_eq!(requests[1].program_name(), "second");
        assert_eq!(requests[1].args_lossy(), vec!["b", "c"]);
    }

    #[test]
    fn mock_returns_scripted_responses_in_order() {
        let mock = MockProcessExecutor::new();
        mock.push_response(ProcessOutput::succeeded().with_stdout("one"));
        mock.push_response(ProcessOutput::failed(3));

        let first = mock.run(ProcessRequest::new("tool")).unwrap();
        assert!(first.success);
        assert_eq!(first.stdout, "one");

        let second = mock.run(ProcessRequest::new("tool")).unwrap();
        assert!(!second.success);
        assert_eq!(second.code, Some(3));
        assert_eq!(second.status_display, "exit status: 3");
    }

    #[test]
    fn mock_program_scripts_take_precedence_over_queue() {
        let mock = MockProcessExecutor::new();
        mock.push_response(ProcessOutput::failed(9));
        mock.push_program_response("hdiutil", ProcessOutput::succeeded());

        let scripted = mock.run(ProcessRequest::new("/usr/bin/hdiutil")).unwrap();
        assert!(scripted.success, "program script must win over the queue");

        // Queue is consumed afterwards for non-matching programs.
        let queued = mock.run(ProcessRequest::new("other")).unwrap();
        assert_eq!(queued.code, Some(9));
    }

    #[test]
    fn mock_program_match_accepts_full_path_or_file_name() {
        let mock = MockProcessExecutor::new();
        mock.push_program_response("/opt/tools/exact", ProcessOutput::succeeded());
        mock.push_program_response("kill", ProcessOutput::failed(1));

        assert!(
            mock.run(ProcessRequest::new("/opt/tools/exact"))
                .unwrap()
                .success
        );
        assert!(!mock.run(ProcessRequest::new("/bin/kill")).unwrap().success);
        assert_eq!(mock.requests_for("kill").len(), 1);
        assert_eq!(mock.requests_for("exact").len(), 1);
    }

    #[test]
    fn mock_scripted_errors_propagate() {
        let mock = MockProcessExecutor::new();
        mock.push_error("boom");

        let err = mock.run(ProcessRequest::new("tool")).unwrap_err();
        assert_eq!(err, "boom");
    }

    #[test]
    fn mock_unexpected_invocation_is_an_error_and_still_recorded() {
        let mock = MockProcessExecutor::new();

        let err = mock
            .run(ProcessRequest::new("surprise").arg("--flag"))
            .unwrap_err();
        assert!(err.contains("unexpected process invocation"), "got: {err}");
        assert!(err.contains("surprise"), "got: {err}");
        assert_eq!(mock.request_count(), 1);
    }

    #[test]
    fn mock_default_response_used_when_no_script_matches() {
        let mock = MockProcessExecutor::new();
        mock.set_default_response(ProcessOutput::succeeded());

        assert!(mock.run(ProcessRequest::new("anything")).unwrap().success);
        assert!(
            mock.run(ProcessRequest::new("anything-else"))
                .unwrap()
                .success
        );
    }

    #[test]
    fn process_request_builder_captures_all_fields() {
        let request = ProcessRequest::new("prog")
            .arg("a")
            .args(["b"])
            .cwd("/tmp")
            .env("KEY", "VALUE")
            .mode(ExecutionMode::Capture)
            .hide_window(true);

        assert_eq!(request.program_name(), "prog");
        assert_eq!(request.args_lossy(), vec!["a", "b"]);
        assert_eq!(request.cwd.as_deref(), Some(Path::new("/tmp")));
        assert_eq!(request.env, vec![("KEY".to_string(), "VALUE".to_string())]);
        assert_eq!(request.mode, ExecutionMode::Capture);
        assert!(request.hide_window);
    }

    // ── SystemProcessExecutor ───────────────────────────────────────────

    fn echo_request(text: &str, mode: ExecutionMode) -> ProcessRequest {
        if cfg!(windows) {
            ProcessRequest::new("cmd")
                .args(["/C", &format!("echo {text}")])
                .mode(mode)
        } else {
            ProcessRequest::new("printf").args(["%s", text]).mode(mode)
        }
    }

    fn failing_request(mode: ExecutionMode) -> ProcessRequest {
        if cfg!(windows) {
            ProcessRequest::new("cmd").args(["/C", "exit 7"]).mode(mode)
        } else {
            ProcessRequest::new("sh").args(["-c", "exit 7"]).mode(mode)
        }
    }

    #[test]
    fn system_executor_capture_returns_stdout_and_status() {
        let output = SystemProcessExecutor
            .run(echo_request("hello-seam", ExecutionMode::Capture))
            .expect("run echo");

        assert!(output.success);
        assert_eq!(output.code, Some(0));
        assert_eq!(output.stdout.trim(), "hello-seam");
    }

    #[test]
    fn system_executor_status_reports_nonzero_exit() {
        let output = SystemProcessExecutor
            .run(failing_request(ExecutionMode::Status))
            .expect("run failing command");

        assert!(!output.success);
        assert_eq!(output.code, Some(7));
        assert!(
            output.status_display.contains('7'),
            "status display should carry the exit code; got {}",
            output.status_display
        );
    }

    #[test]
    fn system_executor_spawn_succeeds_without_waiting() {
        let output = SystemProcessExecutor
            .run(echo_request("spawned", ExecutionMode::Spawn))
            .expect("spawn echo");

        assert!(output.success);
        assert_eq!(output.code, None);
    }

    #[test]
    fn system_executor_missing_binary_is_an_error() {
        let err = SystemProcessExecutor
            .run(
                ProcessRequest::new("gwt_missing_binary_for_seam_test")
                    .mode(ExecutionMode::Capture),
            )
            .unwrap_err();
        assert!(!err.is_empty());
    }

    #[test]
    fn system_executor_applies_env_and_cwd() {
        let dir = tempfile::tempdir().unwrap();
        let request = if cfg!(windows) {
            ProcessRequest::new("cmd")
                .args(["/C", "echo %GWT_SEAM_TEST%& cd"])
                .env("GWT_SEAM_TEST", "seam-value")
                .cwd(dir.path())
                .mode(ExecutionMode::Capture)
        } else {
            ProcessRequest::new("sh")
                .args(["-c", "printf '%s ' \"$GWT_SEAM_TEST\"; pwd"])
                .env("GWT_SEAM_TEST", "seam-value")
                .cwd(dir.path())
                .mode(ExecutionMode::Capture)
        };

        let output = SystemProcessExecutor.run(request).expect("run env probe");
        assert!(output.success);
        assert!(
            output.stdout.contains("seam-value"),
            "env var must reach the child; got {}",
            output.stdout
        );
        let canonical = dir
            .path()
            .canonicalize()
            .unwrap_or_else(|_| dir.path().to_path_buf());
        let stdout_has_dir = output.stdout.contains(&*dir.path().to_string_lossy())
            || output.stdout.contains(&*canonical.to_string_lossy());
        assert!(
            stdout_has_dir,
            "cwd must reach the child; got {}",
            output.stdout
        );
    }
}
