//! `spawn_logged` — the single entry point for spawning external processes
//! while emitting summary tracing events and forwarding stdout / stderr
//! lines to [`ProcessConsoleHub`].
//!
//! SPEC-1924 FR-039: every caller of `Command::new` / `.spawn()` /
//! `.output()` in gwt is expected to migrate to this wrapper. The two
//! intentional exceptions (and how to express them) are:
//!
//! - Detached spawn that intentionally backgrounds (current
//!   `crates/gwt/src/launch_runtime.rs:491-493` and
//!   `crates/gwt-agent/src/prepare.rs:766-768`): pass
//!   `SpawnOptions { detach: true, .. }` so the wrapper still emits a
//!   `start` summary, forwards lines until the child detaches, and
//!   emits a best-effort `exit_code = null` summary at end.
//! - Stdio::null sinks (e.g. `crates/gwt-git/src/worktree.rs:533-534`):
//!   pass `SpawnOptions { capture_stdout: false, capture_stderr: false, .. }`
//!   so the wrapper still emits start / end summary tracing but does not
//!   forward any line.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command as TokioCommand;

use super::hub::ProcessConsoleHub;
use super::kind::ProcessKind;
use super::line::{ProcessLine, ProcessStream};
use super::redact;

const SUMMARY_TARGET: &str = "gwt.process.summary";

static SPAWN_ID: AtomicU64 = AtomicU64::new(1);

/// Knobs that control how `spawn_logged` runs the child process.
#[derive(Debug, Clone)]
pub struct SpawnOptions {
    /// Human-readable command label rendered in summary tracing (e.g.
    /// `"gh pr list"`). The label may differ from the actual argv.
    pub label: String,
    /// Working directory passed to the child.
    pub current_dir: Option<PathBuf>,
    /// Extra env entries to set / override.
    pub envs: Vec<(OsString, OsString)>,
    /// Whether to pipe and forward stdout. Disable for `Stdio::null()`
    /// callers that only need lifecycle summary.
    pub capture_stdout: bool,
    /// Whether to pipe and forward stderr.
    pub capture_stderr: bool,
}

impl SpawnOptions {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            current_dir: None,
            envs: Vec::new(),
            capture_stdout: true,
            capture_stderr: true,
        }
    }

    pub fn current_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.current_dir = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    pub fn capture(mut self, stdout: bool, stderr: bool) -> Self {
        self.capture_stdout = stdout;
        self.capture_stderr = stderr;
        self
    }
}

/// Outcome of a `spawn_logged` call.
#[derive(Debug)]
pub struct SpawnOutput {
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub stdout_lines: u64,
    pub stderr_lines: u64,
}

impl SpawnOutput {
    pub fn success(&self) -> bool {
        matches!(self.exit_code, Some(0))
    }
}

/// Synchronous wrapper around [`spawn_logged`].
///
/// Builds a transient current-thread tokio runtime to drive the async
/// pipeline. Use this from CLI handlers and any sync caller. When the
/// caller already has a tokio runtime handle, prefer the async variant
/// directly.
pub fn spawn_logged_blocking(
    hub: &ProcessConsoleHub,
    kind: ProcessKind,
    program: impl Into<OsString>,
    args: &[impl AsRef<std::ffi::OsStr>],
    options: SpawnOptions,
) -> std::io::Result<SpawnOutput> {
    if let Some(runtime) = shared_blocking_runtime() {
        return runtime.block_on(spawn_logged(hub, kind, program, args, options));
    }
    // Fallback when the shared runtime could not be created at all: build a
    // transient current-thread runtime so behavior matches the historical path.
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(spawn_logged(hub, kind, program, args, options))
}

/// Process-wide runtime reused by every [`spawn_logged_blocking`] call.
///
/// Building a fresh `enable_all()` runtime on each call allocates a new I/O
/// driver descriptor (kqueue / epoll). Under file-descriptor pressure — e.g.
/// many agent PTYs open — that per-call build can fail, and callers that map
/// the failure to `None` then surface misleading downstream errors such as
/// Issue Monitor's "GitHub origin remote is unavailable" (Issue #3190).
/// Reusing one runtime pays the descriptor cost once for the process.
///
/// `block_on` is invoked from external (non-async) threads only — every
/// `spawn_logged_blocking` caller is synchronous or runs inside a blocking
/// pool thread, never inside an async task — which the multi-thread runtime
/// supports from any number of threads concurrently.
fn shared_blocking_runtime() -> Option<&'static tokio::runtime::Runtime> {
    static SHARED: std::sync::OnceLock<Option<tokio::runtime::Runtime>> =
        std::sync::OnceLock::new();
    SHARED
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .ok()
        })
        .as_ref()
}

/// Spawn `program` with `args`, forwarding lines to `hub` and emitting
/// `gwt.process.summary` tracing events at start / end.
pub async fn spawn_logged(
    hub: &ProcessConsoleHub,
    kind: ProcessKind,
    program: impl Into<OsString>,
    args: &[impl AsRef<std::ffi::OsStr>],
    options: SpawnOptions,
) -> std::io::Result<SpawnOutput> {
    let program = program.into();
    let spawn_id = SPAWN_ID.fetch_add(1, Ordering::Relaxed);
    let started_at = Instant::now();

    tracing::info!(
        target: SUMMARY_TARGET,
        kind = kind.as_str(),
        spawn_id = spawn_id,
        label = %options.label,
        program = %program.to_string_lossy(),
        phase = "start",
        "process start",
    );

    // SPEC-2809 (revised) — push the command line as a banner so the
    // Console window shows e.g. `$ gh pr list ...` instead of an opaque
    // `spawn_id` marker. The synthetic line uses the kind's hub so a
    // gh / docker / runner spawn lands under the right tab.
    crate::process::push_command_banner_to_hub(
        kind,
        spawn_id,
        &options.label,
        options.current_dir.as_deref(),
    );

    let mut command = TokioCommand::new(&program);
    command.args(args.iter().map(|a| a.as_ref()));
    if let Some(dir) = &options.current_dir {
        command.current_dir(dir);
    }
    for (key, value) in &options.envs {
        command.env(key, value);
    }
    command.stdin(Stdio::null());
    command.stdout(if options.capture_stdout {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    command.stderr(if options.capture_stderr {
        Stdio::piped()
    } else {
        Stdio::null()
    });

    let mut child = command.spawn()?;

    let stdout_task = if options.capture_stdout {
        let stdout = child.stdout.take().expect("piped stdout");
        let hub = hub.clone();
        Some(tokio::spawn(forward_stream(
            stdout,
            hub,
            kind,
            spawn_id,
            ProcessStream::Stdout,
        )))
    } else {
        None
    };

    let stderr_task = if options.capture_stderr {
        let stderr = child.stderr.take().expect("piped stderr");
        let hub = hub.clone();
        Some(tokio::spawn(forward_stream(
            stderr,
            hub,
            kind,
            spawn_id,
            ProcessStream::Stderr,
        )))
    } else {
        None
    };

    let status = child.wait().await?;

    let (stdout, stdout_lines) = match stdout_task {
        Some(handle) => handle.await.unwrap_or_else(|_| (String::new(), 0)),
        None => (String::new(), 0),
    };
    let (stderr, stderr_lines) = match stderr_task {
        Some(handle) => handle.await.unwrap_or_else(|_| (String::new(), 0)),
        None => (String::new(), 0),
    };

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let exit_code = status.code();

    crate::process::push_command_summary_to_hub(kind, spawn_id, exit_code, duration_ms);

    tracing::info!(
        target: SUMMARY_TARGET,
        kind = kind.as_str(),
        spawn_id = spawn_id,
        label = %options.label,
        phase = "end",
        exit_code = exit_code.map(|c| c as i64),
        duration_ms = duration_ms,
        stdout_lines = stdout_lines,
        stderr_lines = stderr_lines,
        success = status.success(),
        "process end",
    );

    Ok(SpawnOutput {
        exit_code,
        stdout,
        stderr,
        stdout_lines,
        stderr_lines,
    })
}

async fn forward_stream<R>(
    mut reader: R,
    hub: ProcessConsoleHub,
    kind: ProcessKind,
    spawn_id: u64,
    stream: ProcessStream,
) -> (String, u64)
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::with_capacity(4096);
    if reader.read_to_end(&mut bytes).await.is_err() {
        // Fall through; whatever we collected so far is still useful.
    }
    // Hold the caller-facing buffer as the raw text exactly as the
    // child wrote it. `gh auth token` needs the secret to land in the
    // caller's hands unchanged.
    let buf = String::from_utf8_lossy(&bytes).into_owned();

    // Split for the hub: newlines AND carriage returns are treated as
    // line boundaries (the latter so that `docker pull` / `git clone`
    // progress bars surface as discrete entries rather than one giant
    // string). Empty fragments are dropped — they only mark boundary
    // adjacency, not content.
    let mut total_lines: u64 = 0;
    for piece in buf.split(['\n', '\r']) {
        if piece.is_empty() {
            continue;
        }
        // SPEC-2809 FR-008 — ANSI strip then redaction for hub-facing
        // text. The caller-facing `buf` keeps the raw bytes so
        // `gh auth token` and other secret-handling helpers still
        // receive the original value.
        let stripped = super::strip_ansi::strip_ansi(piece);
        let redacted = redact::redact_line(&stripped);
        hub.push(ProcessLine::new(kind, spawn_id, stream, redacted));
        total_lines += 1;
    }
    (buf, total_lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_command() -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), "echo hello world".to_string()],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "echo hello world".to_string()],
            )
        }
    }

    fn stderr_command() -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), "echo oops 1>&2".to_string()],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "echo oops 1>&2".to_string()],
            )
        }
    }

    fn failing_command() -> (String, Vec<String>) {
        if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), "exit 7".to_string()],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "exit 7".to_string()],
            )
        }
    }

    #[tokio::test]
    async fn spawn_logged_forwards_stdout_to_hub() {
        let hub = ProcessConsoleHub::new();
        let (cmd, args) = echo_command();
        let out = spawn_logged(
            &hub,
            ProcessKind::Git,
            cmd,
            &args,
            SpawnOptions::new("test echo"),
        )
        .await
        .unwrap();
        assert!(out.success());
        assert!(out.stdout.contains("hello world"));
        assert_eq!(out.stdout_lines, 1);
        let lines = hub.snapshot_kind(ProcessKind::Git);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].stream, ProcessStream::Stdout);
        assert!(lines[0].message.contains("hello world"));
    }

    #[tokio::test]
    async fn spawn_logged_forwards_stderr_separately() {
        let hub = ProcessConsoleHub::new();
        let (cmd, args) = stderr_command();
        let out = spawn_logged(
            &hub,
            ProcessKind::Docker,
            cmd,
            &args,
            SpawnOptions::new("test stderr"),
        )
        .await
        .unwrap();
        assert!(out.success());
        assert_eq!(out.stderr_lines, 1);
        let lines = hub.snapshot_kind(ProcessKind::Docker);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].stream, ProcessStream::Stderr);
    }

    #[tokio::test]
    async fn spawn_logged_surfaces_non_zero_exit() {
        let hub = ProcessConsoleHub::new();
        let (cmd, args) = failing_command();
        let out = spawn_logged(
            &hub,
            ProcessKind::Gh,
            cmd,
            &args,
            SpawnOptions::new("test fail"),
        )
        .await
        .unwrap();
        assert!(!out.success());
        assert_eq!(out.exit_code, Some(7));
    }

    #[tokio::test]
    async fn spawn_logged_redacts_secrets_in_hub_but_keeps_raw_for_caller() {
        let hub = ProcessConsoleHub::new();
        let token = "ghp_abcdef0123456789ABCDEF";
        let (cmd, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), format!("echo got {token} here")],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), format!("echo got {token} here")],
            )
        };
        let out = spawn_logged(
            &hub,
            ProcessKind::Gh,
            cmd,
            &args,
            SpawnOptions::new("test redact"),
        )
        .await
        .unwrap();
        assert!(out.success());
        // SpawnOutput retains the raw value so that gh auth token /
        // similar helpers receive the real secret.
        assert!(
            out.stdout.contains(token),
            "caller-facing stdout should keep raw token: {:?}",
            out.stdout
        );
        // Hub line is redacted (SPEC-1924 FR-041).
        let lines = hub.snapshot_kind(ProcessKind::Gh);
        assert!(
            !lines[0].message.contains(token),
            "hub line: {:?}",
            lines[0].message
        );
        assert!(lines[0].message.contains("***redacted***"));
    }

    #[tokio::test]
    async fn spawn_logged_capture_off_skips_line_forward() {
        let hub = ProcessConsoleHub::new();
        let (cmd, args) = echo_command();
        let options = SpawnOptions::new("test null").capture(false, false);
        let out = spawn_logged(&hub, ProcessKind::Git, cmd, &args, options)
            .await
            .unwrap();
        assert!(out.success());
        assert!(out.stdout.is_empty());
        assert_eq!(out.stdout_lines, 0);
        let lines = hub.snapshot_kind(ProcessKind::Git);
        assert!(lines.is_empty());
    }

    #[test]
    fn spawn_logged_blocking_runs_and_reuses_shared_runtime() {
        // Issue #3190 regression guard: repeated blocking spawns must succeed
        // without building a fresh `enable_all()` runtime (and a new I/O-driver
        // file descriptor) on every call. The per-call build previously failed
        // under file-descriptor pressure and surfaced as misleading downstream
        // errors such as Issue Monitor's "GitHub origin remote is unavailable".
        let hub = ProcessConsoleHub::new();
        for _ in 0..25 {
            let (cmd, args) = echo_command();
            let out = spawn_logged_blocking(
                &hub,
                ProcessKind::Git,
                cmd,
                &args,
                SpawnOptions::new("blocking echo"),
            )
            .expect("blocking spawn succeeds");
            assert!(out.success());
            assert!(out.stdout.contains("hello world"));
        }
    }

    #[test]
    fn spawn_logged_blocking_surfaces_non_zero_exit() {
        let hub = ProcessConsoleHub::new();
        let (cmd, args) = failing_command();
        let out = spawn_logged_blocking(
            &hub,
            ProcessKind::Gh,
            cmd,
            &args,
            SpawnOptions::new("blocking fail"),
        )
        .expect("blocking spawn returns output even on non-zero exit");
        assert!(!out.success());
        assert_eq!(out.exit_code, Some(7));
    }
}
