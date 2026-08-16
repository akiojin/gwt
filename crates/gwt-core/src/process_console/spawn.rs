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
use std::future::Future;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command as TokioCommand;

use super::hub::ProcessConsoleHub;
use super::kind::ProcessKind;
use super::line::{ProcessLine, ProcessStream};
use super::redact;

const SUMMARY_TARGET: &str = "gwt.process.summary";
const PROCESS_CLEANUP_GRACE: Duration = Duration::from_secs(1);

#[cfg(test)]
thread_local! {
    static TEST_POST_REAP_DELAY_MS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

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
    /// Environment variables removed before spawning.
    pub remove_env: Vec<OsString>,
    /// Whether the child inherits the parent environment.
    pub inherit_env: bool,
    /// Whether to pipe and forward stdout. Disable for `Stdio::null()`
    /// callers that only need lifecycle summary.
    pub capture_stdout: bool,
    /// Whether to pipe and forward stderr.
    pub capture_stderr: bool,
    /// Whether captured output is forwarded to the Process Console hub.
    pub forward_output: bool,
}

impl SpawnOptions {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            current_dir: None,
            envs: Vec::new(),
            remove_env: Vec::new(),
            inherit_env: true,
            capture_stdout: true,
            capture_stderr: true,
            forward_output: true,
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

    pub fn env_remove(mut self, key: impl Into<OsString>) -> Self {
        self.remove_env.push(key.into());
        self
    }

    pub fn inherit_env(mut self, inherit: bool) -> Self {
        self.inherit_env = inherit;
        self
    }

    pub fn capture(mut self, stdout: bool, stderr: bool) -> Self {
        self.capture_stdout = stdout;
        self.capture_stderr = stderr;
        self
    }

    pub fn forward_output(mut self, forward: bool) -> Self {
        self.forward_output = forward;
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
    if let Some(deadline) = crate::operation_deadline::current() {
        return spawn_logged_blocking_with_deadline(hub, kind, program, args, options, deadline);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(spawn_logged(hub, kind, program, args, options))
}

/// Synchronous wrapper around [`spawn_logged_with_deadline`].
pub fn spawn_logged_blocking_with_deadline(
    hub: &ProcessConsoleHub,
    kind: ProcessKind,
    program: impl Into<OsString>,
    args: &[impl AsRef<std::ffi::OsStr>],
    options: SpawnOptions,
    deadline: Instant,
) -> std::io::Result<SpawnOutput> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(spawn_logged_with_deadline(
        hub, kind, program, args, options, deadline,
    ))
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
    spawn_logged_inner(hub, kind, program, args, options, None).await
}

/// Spawn a logged child under one absolute deadline.
///
/// The deadline covers process completion and stdout/stderr EOF. On expiry the
/// dedicated process tree is terminated and the direct child is reaped before
/// this function returns.
pub async fn spawn_logged_with_deadline(
    hub: &ProcessConsoleHub,
    kind: ProcessKind,
    program: impl Into<OsString>,
    args: &[impl AsRef<std::ffi::OsStr>],
    options: SpawnOptions,
    deadline: Instant,
) -> std::io::Result<SpawnOutput> {
    spawn_logged_inner(hub, kind, program, args, options, Some(deadline)).await
}

async fn spawn_logged_inner(
    hub: &ProcessConsoleHub,
    kind: ProcessKind,
    program: impl Into<OsString>,
    args: &[impl AsRef<std::ffi::OsStr>],
    options: SpawnOptions,
    deadline: Option<Instant>,
) -> std::io::Result<SpawnOutput> {
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
        return Err(deadline_error());
    }
    // Issue #3604 AC-3: an exhausted GitHub budget refuses the call here, so a
    // rate-limited window stops producing spawns, log noise, and generic
    // "network error" reports until its measured reset passes.
    let gh_quota = matches!(kind, ProcessKind::Gh).then(|| gh_arg_strings(args));
    if let Some(args) = &gh_quota {
        if let Some(detail) = crate::github_quota::suppressed_spawn_detail(
            crate::github_quota::global(),
            args,
            chrono::Utc::now(),
        ) {
            tracing::warn!(
                target: SUMMARY_TARGET,
                kind = kind.as_str(),
                label = %options.label,
                detail = %detail,
                "gh call suppressed: GitHub budget exhausted"
            );
            return Err(std::io::Error::other(detail));
        }
    }
    let program = program.into();
    let spawn_id = SPAWN_ID.fetch_add(1, Ordering::Relaxed);
    let started_at = Instant::now();

    trace_process_start(kind, spawn_id, &options, &program);

    // SPEC-2809 (revised) — push the command line as a banner so the
    // Console window shows e.g. `$ gh pr list ...` instead of an opaque
    // `spawn_id` marker. The synthetic line uses the kind's hub so a
    // gh / docker / runner spawn lands under the right tab.
    if options.forward_output {
        push_command_banner_to_hub(
            hub,
            kind,
            spawn_id,
            &options.label,
            options.current_dir.as_deref(),
        );
    }

    let mut request = crate::process::ProcessPlanRequest::new(&program)
        .args(args.iter().map(|arg| arg.as_ref()))
        .inherit_env(options.inherit_env);
    if let Some(dir) = &options.current_dir {
        request = request.current_dir(dir);
    }
    for (key, value) in &options.envs {
        request = request.env(key, value);
    }
    for key in &options.remove_env {
        request = request.env_remove(key);
    }
    let mut command = match crate::process::resolved_tokio_command(request) {
        Ok(command) => command,
        Err(error) => {
            let error = process_resolution_io_error(error);
            finish_failed_launch(
                hub,
                kind,
                spawn_id,
                &options.label,
                started_at,
                &error,
                options.forward_output,
            );
            return Err(error);
        }
    };
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

    let mut process_tree = match ChildProcessTree::prepare(&mut command, deadline.is_some()) {
        Ok(process_tree) => process_tree,
        Err(error) => {
            finish_failed_launch(
                hub,
                kind,
                spawn_id,
                &options.label,
                started_at,
                &error,
                options.forward_output,
            );
            return Err(error);
        }
    };

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            finish_failed_launch(
                hub,
                kind,
                spawn_id,
                &options.label,
                started_at,
                &error,
                options.forward_output,
            );
            return Err(error);
        }
    };
    if let Some(pid) = deadline.and_then(|_| child.id()) {
        if let Err(error) = process_tree.after_spawn(pid) {
            let _ = cleanup_child_process(&mut process_tree, &mut child, deadline).await;
            finish_failed_launch(
                hub,
                kind,
                spawn_id,
                &options.label,
                started_at,
                &error,
                options.forward_output,
            );
            return Err(error);
        }
    }
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let forward_output = options.forward_output;
    let collection_deadline = deadline.map(deadline_before_cleanup_reserve);
    let collected = {
        let collect = async {
            let stdout_future = async move {
                Ok::<_, std::io::Error>(match stdout {
                    Some(stdout) => {
                        forward_stream(
                            stdout,
                            hub.clone(),
                            kind,
                            spawn_id,
                            ProcessStream::Stdout,
                            forward_output,
                        )
                        .await
                    }
                    None => (String::new(), 0),
                })
            };
            let stderr_future = async move {
                Ok::<_, std::io::Error>(match stderr {
                    Some(stderr) => {
                        forward_stream(
                            stderr,
                            hub.clone(),
                            kind,
                            spawn_id,
                            ProcessStream::Stderr,
                            forward_output,
                        )
                        .await
                    }
                    None => (String::new(), 0),
                })
            };
            tokio::try_join!(child.wait(), stdout_future, stderr_future)
        };
        tokio::pin!(collect);
        match collection_deadline {
            Some(deadline) => tokio::time::timeout_at(deadline.into(), &mut collect)
                .await
                .ok(),
            None => Some(collect.await),
        }
    };

    let Some(collected) = collected else {
        if !cleanup_child_process(&mut process_tree, &mut child, deadline).await {
            trace_cleanup_grace_exceeded(spawn_id, options.forward_output);
        }
        let duration_ms = started_at.elapsed().as_millis() as u64;
        if options.forward_output {
            push_command_summary_to_hub(hub, kind, spawn_id, None, duration_ms);
        }
        trace_process_end(
            kind,
            spawn_id,
            &options,
            None,
            duration_ms,
            0,
            0,
            false,
            true,
        );
        return Err(deadline_error());
    };
    let (status, (stdout, stdout_lines), (stderr, stderr_lines)) = match collected {
        Ok(collected) => collected,
        Err(error) => {
            if !cleanup_child_process(&mut process_tree, &mut child, deadline).await {
                trace_cleanup_grace_exceeded(spawn_id, options.forward_output);
            }
            finish_failed_launch(
                hub,
                kind,
                spawn_id,
                &options.label,
                started_at,
                &error,
                options.forward_output,
            );
            return Err(error);
        }
    };
    if let Err(error) = process_tree.release_without_termination() {
        finish_failed_launch(
            hub,
            kind,
            spawn_id,
            &options.label,
            started_at,
            &error,
            options.forward_output,
        );
        return Err(error);
    }

    let duration_ms = started_at.elapsed().as_millis() as u64;
    let exit_code = status.code();

    if options.forward_output {
        push_command_summary_to_hub(hub, kind, spawn_id, exit_code, duration_ms);
    }
    trace_process_end(
        kind,
        spawn_id,
        &options,
        exit_code,
        duration_ms,
        stdout_lines,
        stderr_lines,
        status.success(),
        false,
    );

    let stderr = if let Some(gh_args) = &gh_quota {
        reconcile_github_quota(
            hub,
            &program,
            &options,
            gh_args,
            status.success(),
            stderr,
            deadline,
        )
        .await
    } else {
        stderr
    };

    Ok(SpawnOutput {
        exit_code,
        stdout,
        stderr,
        stdout_lines,
        stderr_lines,
    })
}

fn gh_arg_strings(args: &[impl AsRef<std::ffi::OsStr>]) -> Vec<String> {
    args.iter()
        .map(|arg| arg.as_ref().to_string_lossy().into_owned())
        .collect()
}

/// Issue #3604 AC-1 / AC-2: turn a bare `gh` rate-limit refusal into an
/// identified failure that carries its reset window, and remember the window so
/// the pre-spawn gate can suppress the calls that would follow it.
///
/// `gh` never prints the reset time, so the window comes from `gh api
/// rate_limit` — a free endpoint that spends neither budget, which is also why
/// this reconcile is a structural no-op for [`crate::github_quota::GitHubQuota::Free`]
/// argv and therefore cannot recurse.
async fn reconcile_github_quota(
    hub: &ProcessConsoleHub,
    program: &OsString,
    options: &SpawnOptions,
    args: &[String],
    success: bool,
    stderr: String,
    deadline: Option<Instant>,
) -> String {
    use crate::github_quota::{self, GitHubQuota};

    let quota = github_quota::classify_gh_args(args);
    if quota == GitHubQuota::Free {
        return stderr;
    }
    if success {
        // The budget answered, so any recorded block over-estimated its window.
        github_quota::global().record_success(quota);
        return stderr;
    }
    if !github_quota::is_rate_limit_stderr(&stderr) {
        return stderr;
    }

    let now = chrono::Utc::now();
    let probe = probe_rate_limit(hub, program, options, quota, deadline).await;
    let block = github_quota::block_from_probe(quota, probe, now);
    let annotated = github_quota::annotate_rate_limited_stderr(&block, &stderr, now);
    github_quota::global().record_exhaustion(block);
    annotated
}

async fn probe_rate_limit(
    hub: &ProcessConsoleHub,
    program: &OsString,
    options: &SpawnOptions,
    quota: crate::github_quota::GitHubQuota,
    deadline: Option<Instant>,
) -> Option<crate::github_quota::RateLimitBlock> {
    let mut probe_options = SpawnOptions::new("gh api rate_limit").forward_output(false);
    if let Some(dir) = &options.current_dir {
        probe_options = probe_options.current_dir(dir.clone());
    }
    // Boxed so the (runtime-unreachable) self-reference stays finitely sized.
    let output = Box::pin(spawn_logged_inner(
        hub,
        ProcessKind::Gh,
        program.clone(),
        crate::github_quota::RATE_LIMIT_PROBE_ARGS,
        probe_options,
        // The probe must not outlive its caller's operation budget: a scan
        // stage that already ran out of time cannot afford one more spawn.
        deadline,
    ))
    .await
    .ok()?;
    if !output.success() {
        return None;
    }
    crate::github_quota::parse_rate_limit_probe(&output.stdout, quota)
}

fn trace_process_start(
    kind: ProcessKind,
    spawn_id: u64,
    options: &SpawnOptions,
    program: &std::ffi::OsStr,
) {
    if options.forward_output {
        tracing::info!(
            target: SUMMARY_TARGET,
            kind = kind.as_str(),
            spawn_id,
            label = %options.label,
            program = %program.to_string_lossy(),
            phase = "start",
            "process start",
        );
    } else {
        // Silent process execution is intentionally invisible to every UI
        // surface. Keep only non-sensitive correlation metadata at Debug.
        tracing::debug!(
            target: SUMMARY_TARGET,
            kind = kind.as_str(),
            spawn_id,
            phase = "start",
            "silent process start",
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn trace_process_end(
    kind: ProcessKind,
    spawn_id: u64,
    options: &SpawnOptions,
    exit_code: Option<i32>,
    duration_ms: u64,
    stdout_lines: u64,
    stderr_lines: u64,
    success: bool,
    timed_out: bool,
) {
    if options.forward_output {
        tracing::info!(
            target: SUMMARY_TARGET,
            kind = kind.as_str(),
            spawn_id,
            label = %options.label,
            phase = "end",
            exit_code = exit_code.map(i64::from),
            duration_ms,
            stdout_lines,
            stderr_lines,
            success,
            timed_out,
            "process end",
        );
    } else {
        tracing::debug!(
            target: SUMMARY_TARGET,
            kind = kind.as_str(),
            spawn_id,
            phase = "end",
            exit_code = exit_code.map(i64::from),
            duration_ms,
            success,
            timed_out,
            "silent process end",
        );
    }
}

fn trace_cleanup_grace_exceeded(spawn_id: u64, forward_output: bool) {
    if forward_output {
        tracing::warn!(
            target: SUMMARY_TARGET,
            spawn_id,
            cleanup_grace_ms = PROCESS_CLEANUP_GRACE.as_millis() as u64,
            "process cleanup exceeded its grace period",
        );
    } else {
        tracing::debug!(
            target: SUMMARY_TARGET,
            spawn_id,
            "silent process cleanup exceeded its grace period",
        );
    }
}

fn push_command_banner_to_hub(
    hub: &ProcessConsoleHub,
    kind: ProcessKind,
    spawn_id: u64,
    label: &str,
    current_dir: Option<&std::path::Path>,
) {
    let banner = match current_dir {
        Some(dir) => format!("$ {label} (cwd={})", dir.display()),
        None => format!("$ {label}"),
    };
    hub.push(ProcessLine::new(
        kind,
        spawn_id,
        ProcessStream::Stdout,
        banner,
    ));
}

fn push_command_summary_to_hub(
    hub: &ProcessConsoleHub,
    kind: ProcessKind,
    spawn_id: u64,
    exit_code: Option<i32>,
    duration_ms: u64,
) {
    let exit = exit_code.map_or_else(|| "?".to_string(), |code| code.to_string());
    hub.push(ProcessLine::new(
        kind,
        spawn_id,
        ProcessStream::Stdout,
        format!("→ exit={exit} ({duration_ms}ms)"),
    ));
}

fn finish_failed_launch(
    hub: &ProcessConsoleHub,
    kind: ProcessKind,
    spawn_id: u64,
    label: &str,
    started_at: Instant,
    error: &std::io::Error,
    forward_output: bool,
) {
    let duration_ms = started_at.elapsed().as_millis() as u64;
    if forward_output {
        hub.push(ProcessLine::new(
            kind,
            spawn_id,
            ProcessStream::Stderr,
            redact::redact_line(&format!("process launch failed: {error}")),
        ));
        push_command_summary_to_hub(hub, kind, spawn_id, None, duration_ms);
        tracing::info!(
            target: SUMMARY_TARGET,
            kind = kind.as_str(),
            spawn_id = spawn_id,
            label = %label,
            phase = "end",
            exit_code = Option::<i64>::None,
            duration_ms = duration_ms,
            stdout_lines = 0_u64,
            stderr_lines = 1_u64,
            success = false,
            error = %error,
            "process end",
        );
    } else {
        tracing::debug!(
            target: SUMMARY_TARGET,
            kind = kind.as_str(),
            spawn_id,
            phase = "end",
            duration_ms,
            success = false,
            "silent process launch failed",
        );
    }
}

fn process_resolution_io_error(error: crate::process::ProcessResolveFailure) -> std::io::Error {
    let kind = match error.kind {
        crate::process::ProcessResolveFailureKind::NotFound => std::io::ErrorKind::NotFound,
        crate::process::ProcessResolveFailureKind::UnsafeExecutable => {
            std::io::ErrorKind::PermissionDenied
        }
    };
    std::io::Error::new(kind, error)
}

fn deadline_error() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::TimedOut, "process deadline expired")
}

fn deadline_before_cleanup_reserve(deadline: Instant) -> Instant {
    let now = Instant::now();
    let remaining = deadline.saturating_duration_since(now);
    let reserve = remaining.mul_f32(0.25).min(PROCESS_CLEANUP_GRACE);
    deadline
        .checked_sub(reserve)
        .map(|candidate| candidate.max(now))
        .unwrap_or(now)
}

async fn cleanup_child_process(
    process_tree: &mut ChildProcessTree,
    child: &mut tokio::process::Child,
    deadline: Option<Instant>,
) -> bool {
    match deadline {
        Some(deadline) => {
            cleanup_child_process_after_tree_termination_until(
                deadline,
                process_tree.terminate(),
                child,
            )
            .await
        }
        None => {
            cleanup_child_process_after_tree_termination(
                PROCESS_CLEANUP_GRACE,
                process_tree.terminate(),
                child,
            )
            .await
        }
    }
}

async fn cleanup_child_process_after_tree_termination<F>(
    grace: Duration,
    tree_termination: F,
    child: &mut tokio::process::Child,
) -> bool
where
    F: Future<Output = ()>,
{
    run_cleanup_with_grace(grace, async {
        tree_termination.await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        test_post_reap_delay().await;
    })
    .await
}

async fn cleanup_child_process_after_tree_termination_until<F>(
    deadline: Instant,
    tree_termination: F,
    child: &mut tokio::process::Child,
) -> bool
where
    F: Future<Output = ()>,
{
    run_cleanup_until(deadline, async {
        tree_termination.await;
        let _ = child.start_kill();
        let _ = child.wait().await;
        test_post_reap_delay().await;
    })
    .await
}

#[cfg(test)]
async fn test_post_reap_delay() {
    let delay_ms = TEST_POST_REAP_DELAY_MS.with(std::cell::Cell::get);
    if delay_ms > 0 {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }
}

#[cfg(not(test))]
async fn test_post_reap_delay() {}

async fn run_cleanup_with_grace<F>(grace: Duration, cleanup: F) -> bool
where
    F: Future<Output = ()>,
{
    tokio::time::timeout(grace, cleanup).await.is_ok()
}

async fn run_cleanup_until<F>(deadline: Instant, cleanup: F) -> bool
where
    F: Future<Output = ()>,
{
    tokio::time::timeout_at(deadline.into(), cleanup)
        .await
        .is_ok()
}

struct ChildProcessTree {
    #[cfg(unix)]
    pid: Option<u32>,
    #[cfg(windows)]
    job: Option<crate::process_tree::WindowsJobObject>,
    #[cfg(not(any(unix, windows)))]
    pid: Option<u32>,
}

impl ChildProcessTree {
    fn prepare(command: &mut TokioCommand, deadline_enabled: bool) -> std::io::Result<Self> {
        if deadline_enabled {
            command.kill_on_drop(true);
        }
        #[cfg(unix)]
        {
            if deadline_enabled {
                command.process_group(0);
            }
            Ok(Self { pid: None })
        }
        #[cfg(windows)]
        {
            let job = if deadline_enabled {
                let job =
                    crate::process_tree::WindowsJobObject::new().map_err(windows_job_io_error)?;
                crate::process_tree::WindowsJobObject::configure_suspended(command.as_std_mut());
                Some(job)
            } else {
                None
            };
            Ok(Self { job })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(Self { pid: None })
        }
    }

    fn after_spawn(&mut self, pid: u32) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            self.pid = Some(pid);
            Ok(())
        }
        #[cfg(windows)]
        {
            self.job
                .as_mut()
                .expect("deadline process owns a Windows Job")
                .assign_and_resume(pid)
                .map_err(windows_job_io_error)
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.pid = Some(pid);
            Ok(())
        }
    }

    async fn terminate(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid.take() {
            terminate_process_tree(pid).await;
        }
        #[cfg(windows)]
        if let Some(mut job) = self.job.take() {
            let _ = job.terminate();
        }
        #[cfg(not(any(unix, windows)))]
        if let Some(pid) = self.pid.take() {
            terminate_process_tree(pid).await;
        }
    }

    fn terminate_on_drop(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid.take() {
            terminate_process_tree_on_drop(pid);
        }
        #[cfg(windows)]
        if let Some(mut job) = self.job.take() {
            let _ = job.terminate();
        }
        #[cfg(not(any(unix, windows)))]
        if let Some(pid) = self.pid.take() {
            terminate_process_tree_on_drop(pid);
        }
    }

    fn release_without_termination(&mut self) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            self.pid = None;
            Ok(())
        }
        #[cfg(windows)]
        {
            if let Some(job) = self.job.take() {
                job.release_without_termination()
                    .map_err(windows_job_io_error)?;
            }
            Ok(())
        }
        #[cfg(not(any(unix, windows)))]
        {
            self.pid = None;
            Ok(())
        }
    }
}

impl Drop for ChildProcessTree {
    fn drop(&mut self) {
        self.terminate_on_drop();
    }
}

#[cfg(unix)]
async fn terminate_process_tree(pid: u32) {
    terminate_process_tree_on_drop(pid);
}

#[cfg(unix)]
fn terminate_process_tree_on_drop(pid: u32) {
    let process_group = -(pid as libc::pid_t);
    // SAFETY: the deadline command was placed in its own process group and a
    // negative pid targets only that group.
    unsafe {
        libc::kill(process_group, libc::SIGKILL);
    }
}

#[cfg(not(any(unix, windows)))]
async fn terminate_process_tree(_pid: u32) {}

#[cfg(not(any(unix, windows)))]
fn terminate_process_tree_on_drop(_pid: u32) {}

#[cfg(windows)]
fn windows_job_io_error(error: crate::process_tree::WindowsJobError) -> std::io::Error {
    std::io::Error::other(error)
}

async fn forward_stream<R>(
    mut reader: R,
    hub: ProcessConsoleHub,
    kind: ProcessKind,
    spawn_id: u64,
    stream: ProcessStream,
    forward_output: bool,
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
    if forward_output {
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
    }
    (buf, total_lines)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io::Write;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use super::*;

    struct PostReapDelayGuard(u64);

    impl PostReapDelayGuard {
        fn set(delay: Duration) -> Self {
            let previous = TEST_POST_REAP_DELAY_MS.with(|slot| {
                let previous = slot.get();
                slot.set(delay.as_millis() as u64);
                previous
            });
            Self(previous)
        }
    }

    impl Drop for PostReapDelayGuard {
        fn drop(&mut self) {
            TEST_POST_REAP_DELAY_MS.with(|slot| slot.set(self.0));
        }
    }

    #[derive(Clone, Default)]
    struct CapturedTrace(Arc<Mutex<Vec<u8>>>);

    impl CapturedTrace {
        fn contents(&self) -> String {
            String::from_utf8_lossy(
                &self
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
            .into_owned()
        }
    }

    impl Write for CapturedTrace {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for CapturedTrace {
        type Writer = Self;

        fn make_writer(&'writer self) -> Self::Writer {
            self.clone()
        }
    }

    #[tokio::test]
    async fn cleanup_future_is_abandoned_after_grace() {
        let started = std::time::Instant::now();
        let completed = tokio::time::timeout(
            Duration::from_millis(200),
            run_cleanup_with_grace(Duration::from_millis(20), std::future::pending()),
        )
        .await
        .expect("cleanup grace must bound a stalled cleanup future");

        assert!(!completed, "stalled cleanup must report incomplete");
        assert!(started.elapsed() < Duration::from_millis(150));
    }

    #[tokio::test]
    async fn stalled_tree_termination_uses_cleanup_grace() {
        let (program, args) = if cfg!(windows) {
            (
                "ping",
                vec!["-n".to_string(), "30".to_string(), "127.0.0.1".to_string()],
            )
        } else {
            ("sleep", vec!["30".to_string()])
        };
        #[allow(clippy::disallowed_methods)]
        let mut command = TokioCommand::new(program);
        crate::process::configure_hidden_tokio_command(&mut command);
        command
            .args(args)
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = command.spawn().expect("spawn cleanup test child");
        let started = std::time::Instant::now();

        let completed = tokio::time::timeout(
            Duration::from_millis(200),
            cleanup_child_process_after_tree_termination(
                Duration::from_millis(20),
                std::future::pending(),
                &mut child,
            ),
        )
        .await
        .expect("tree termination must be inside the cleanup grace");

        assert!(
            !completed,
            "stalled tree termination must report incomplete"
        );
        assert!(started.elapsed() < Duration::from_millis(150));
        let _ = child.start_kill();
        let _ = child.wait().await;
    }

    #[cfg(any(unix, windows))]
    #[tokio::test(flavor = "current_thread")]
    async fn deadline_cleanup_does_not_extend_the_absolute_hard_cap() {
        let directory = tempfile::tempdir().expect("tempdir");
        let parent_file = directory.path().join("hard-cap-parent.pid");
        let descendant_file = directory.path().join("hard-cap-descendant.pid");
        #[cfg(windows)]
        let (program, args, budget, delay) = {
            let script = format!(
                "Set-Content -Path '{}' -Value $PID -Encoding ascii; \
                 $child = Start-Process ping -ArgumentList '-n','60','127.0.0.1' \
                 -PassThru -WindowStyle Hidden; \
                 Set-Content -Path '{}' -Value $child.Id -Encoding ascii; \
                 Start-Sleep -Seconds 60",
                parent_file.display(),
                descendant_file.display(),
            );
            (
                "powershell".to_string(),
                vec!["-NoProfile".to_string(), "-Command".to_string(), script],
                WINDOWS_PROCESS_TREE_FIXTURE_BUDGET,
                Duration::from_millis(1_200),
            )
        };
        #[cfg(unix)]
        let (program, args, budget, delay) = (
            "sh".to_string(),
            vec![
                "-c".to_string(),
                "echo $$ > \"$1\"; sleep 60 & echo $! > \"$2\"; wait".to_string(),
                "gwt-hard-cap".to_string(),
                parent_file.to_string_lossy().into_owned(),
                descendant_file.to_string_lossy().into_owned(),
            ],
            Duration::from_millis(700),
            Duration::from_millis(600),
        );
        let _delay = PostReapDelayGuard::set(delay);
        let started = Instant::now();
        let deadline = started + budget;

        let error = spawn_logged_with_deadline(
            &ProcessConsoleHub::new(),
            ProcessKind::IndexRunner,
            program,
            &args,
            SpawnOptions::new("absolute deadline cleanup")
                .forward_output(false)
                // Fixtures run PowerShell while other tests in the same binary
                // redirect HOME / LOCALAPPDATA process-wide. Without a usable
                // cache location PowerShell falls back to writing its module
                // analysis cache relative to the CWD, which would litter the
                // crate directory. Pin the child to the fixture temp directory
                // so any such fallback is cleaned up with it.
                .current_dir(directory.path()),
            deadline,
        )
        .await
        .expect_err("fixture tree must time out");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() <= budget + Duration::from_millis(250),
            "cleanup extended the absolute deadline: budget={budget:?} elapsed={:?}",
            started.elapsed()
        );
        #[cfg(windows)]
        {
            let parent = wait_for_pid_file_windows(&parent_file);
            let descendant = wait_for_pid_file_windows(&descendant_file);
            assert!(!process_is_alive_windows(parent), "root survived cleanup");
            assert!(
                !process_is_alive_windows(descendant),
                "descendant survived cleanup"
            );
        }
        #[cfg(unix)]
        {
            let parent = read_pid(&parent_file);
            let descendant = read_pid(&descendant_file);
            assert!(!process_is_alive(parent), "root survived cleanup");
            assert!(!process_is_alive(descendant), "descendant survived cleanup");
        }
    }

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
        assert_eq!(lines.len(), 3);
        assert!(lines[0].message.starts_with("$ test echo"));
        assert_eq!(lines[1].stream, ProcessStream::Stdout);
        assert!(lines[1].message.contains("hello world"));
        assert!(lines[2].message.starts_with("→ exit=0"));
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
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].stream, ProcessStream::Stderr);
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
    async fn spawn_logged_closes_the_console_lifecycle_when_launch_fails() {
        let hub = ProcessConsoleHub::new();
        let error = spawn_logged(
            &hub,
            ProcessKind::AgentBootstrap,
            "this-binary-does-not-exist-gwt-process-console-test",
            &[] as &[&str],
            SpawnOptions::new("missing executable"),
        )
        .await
        .expect_err("missing executable must fail");

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
        let lines = hub.snapshot_kind(ProcessKind::AgentBootstrap);
        assert_eq!(
            lines.len(),
            3,
            "banner, diagnostic, and footer must be emitted"
        );
        assert!(lines[0].message.starts_with("$ missing executable"));
        assert_eq!(lines[1].stream, ProcessStream::Stderr);
        assert!(lines[1].message.contains("process launch failed"));
        assert_eq!(lines[2].stream, ProcessStream::Stdout);
        assert!(lines[2].message.starts_with("→ exit=?"));
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
        let output_line = lines
            .iter()
            .find(|line| line.message.contains("***redacted***"))
            .expect("redacted child output line");
        assert!(
            !output_line.message.contains(token),
            "hub line: {:?}",
            output_line.message
        );
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
        assert_eq!(lines.len(), 2, "capture off keeps lifecycle only");
        assert!(lines[0].message.starts_with("$ test null"));
        assert!(lines[1].message.starts_with("→ exit=0"));
    }

    #[test]
    fn spawn_options_capture_environment_contract() {
        let options = SpawnOptions::new("env contract")
            .env("KEEP", "preserved")
            .env_remove("DROP")
            .inherit_env(false);

        assert_eq!(
            options.envs,
            vec![(OsString::from("KEEP"), OsString::from("preserved"))]
        );
        assert_eq!(options.remove_env, vec![OsString::from("DROP")]);
        assert!(!options.inherit_env);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_logged_can_clear_inherited_environment() {
        let output = spawn_logged(
            &ProcessConsoleHub::new(),
            ProcessKind::Git,
            "/usr/bin/env",
            &[] as &[&str],
            SpawnOptions::new("cleared environment")
                .env("GWT_PROCESS_KEEP", "preserved")
                .inherit_env(false),
        )
        .await
        .expect("spawn env with a cleared inherited environment");

        assert!(output.success());
        assert!(output.stdout.contains("GWT_PROCESS_KEEP=preserved"));
        assert!(
            !output.stdout.lines().any(|line| line.starts_with("PATH=")),
            "inherited PATH must not leak into the child: {}",
            output.stdout
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_logged_can_remove_one_inherited_environment_variable() {
        let output = spawn_logged(
            &ProcessConsoleHub::new(),
            ProcessKind::Git,
            "/usr/bin/env",
            &[] as &[&str],
            SpawnOptions::new("removed environment").env_remove("PATH"),
        )
        .await
        .expect("spawn env with PATH removed");

        assert!(output.success());
        assert!(
            !output.stdout.lines().any(|line| line.starts_with("PATH=")),
            "removed PATH must not reach the child: {}",
            output.stdout
        );
    }

    #[tokio::test]
    async fn spawn_logged_can_capture_sensitive_stdout_without_forwarding_it() {
        let hub = ProcessConsoleHub::new();
        let sensitive = "https://user:unredacted-secret@github.com/akiojin/gwt.git";
        let (cmd, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), format!("echo {sensitive}")],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), format!("echo {sensitive}")],
            )
        };
        let out = spawn_logged(
            &hub,
            ProcessKind::Git,
            cmd,
            &args,
            SpawnOptions::new("sensitive git config").forward_output(false),
        )
        .await
        .expect("capture sensitive output");

        assert!(out.stdout.contains(sensitive));
        assert_eq!(out.stdout_lines, 0);
        assert!(hub.snapshot_kind(ProcessKind::Git).is_empty());
    }

    #[test]
    fn silent_success_timeout_and_spawn_failure_emit_no_hub_or_ui_events() {
        use tracing_subscriber::prelude::*;

        let supplied_hub = ProcessConsoleHub::new();
        let candidate_global = ProcessConsoleHub::new();
        let installed_global = super::super::hub::set_global(candidate_global.clone());
        let global_before = candidate_global
            .snapshot_kind(ProcessKind::IndexRunner)
            .len();
        let (ui_tx, mut ui_rx) = tokio::sync::mpsc::unbounded_channel();
        let trace = CapturedTrace::default();
        let subscriber = tracing_subscriber::registry()
            .with(crate::logging::ui_forwarder::UiForwarderLayer::new(ui_tx))
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_ansi(false)
                    .with_writer(trace.clone()),
            );
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");

        tracing::subscriber::with_default(subscriber, || {
            runtime.block_on(async {
                let (success_program, success_args) = echo_command();
                spawn_logged(
                    &supplied_hub,
                    ProcessKind::IndexRunner,
                    success_program,
                    &success_args,
                    SpawnOptions::new("secret label ghp_abcdef0123456789").forward_output(false),
                )
                .await
                .expect("silent success");

                let (timeout_program, timeout_args) = if cfg!(windows) {
                    (
                        "cmd".to_string(),
                        vec!["/C".to_string(), "ping -n 4 127.0.0.1 >NUL".to_string()],
                    )
                } else {
                    (
                        "sh".to_string(),
                        vec!["-c".to_string(), "sleep 3".to_string()],
                    )
                };
                let timeout = spawn_logged_with_deadline(
                    &supplied_hub,
                    ProcessKind::IndexRunner,
                    timeout_program,
                    &timeout_args,
                    SpawnOptions::new("private timeout path C:\\secret").forward_output(false),
                    Instant::now() + Duration::from_millis(150),
                )
                .await
                .expect_err("silent timeout");
                assert_eq!(timeout.kind(), std::io::ErrorKind::TimedOut);

                spawn_logged(
                    &supplied_hub,
                    ProcessKind::IndexRunner,
                    "this-binary-does-not-exist-silent-process-test",
                    &[] as &[&str],
                    SpawnOptions::new("private missing executable").forward_output(false),
                )
                .await
                .expect_err("silent spawn failure");
            });
        });

        assert!(
            supplied_hub
                .snapshot_kind(ProcessKind::IndexRunner)
                .is_empty(),
            "silent lifecycle must not reach the supplied hub"
        );
        if installed_global {
            assert_eq!(
                candidate_global
                    .snapshot_kind(ProcessKind::IndexRunner)
                    .len(),
                global_before,
                "silent lifecycle must not reach the global hub"
            );
        }
        assert!(
            ui_rx.try_recv().is_err(),
            "Debug-only silent lifecycle must not reach UiForwarder"
        );
        let trace = trace.contents();
        assert!(trace.contains("silent process"));
        for sensitive in [
            "ghp_abcdef0123456789",
            r"C:\secret",
            "this-binary-does-not-exist-silent-process-test",
        ] {
            assert!(
                !trace.contains(sensitive),
                "silent Debug lifecycle leaked sensitive context: {trace}"
            );
        }
    }

    #[tokio::test]
    async fn spawn_logged_deadline_succeeds_before_expiry() {
        let hub = ProcessConsoleHub::new();
        let (cmd, args) = echo_command();
        let out = spawn_logged_with_deadline(
            &hub,
            ProcessKind::Git,
            cmd,
            &args,
            SpawnOptions::new("test deadline echo"),
            std::time::Instant::now() + Duration::from_secs(2),
        )
        .await
        .expect("command before deadline");
        assert!(out.success());
        assert!(out.stdout.contains("hello world"));
    }

    #[test]
    fn spawn_logged_blocking_stops_finite_sleep_at_scoped_deadline() {
        let (program, args) = if cfg!(windows) {
            (
                "cmd".to_string(),
                vec!["/C".to_string(), "ping -n 3 127.0.0.1 >NUL".to_string()],
            )
        } else {
            (
                "sh".to_string(),
                vec!["-c".to_string(), "sleep 2".to_string()],
            )
        };
        let started = std::time::Instant::now();
        let _deadline = crate::operation_deadline::ScopedOperationDeadline::enter(
            started + Duration::from_millis(150),
        );

        let error = spawn_logged_blocking(
            &ProcessConsoleHub::new(),
            ProcessKind::Gh,
            program,
            &args,
            SpawnOptions::new("test scoped deadline sleep"),
        )
        .expect_err("scoped deadline must stop a finite sleep before completion");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(
            started.elapsed() < Duration::from_millis(1_500),
            "finite sleep outlived the scoped deadline: {:?}",
            started.elapsed()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn expired_deadline_does_not_spawn_child() {
        let directory = tempfile::tempdir().expect("tempdir");
        let sentinel = directory.path().join("spawned");
        let args = vec![
            "-c".to_string(),
            "touch \"$1\"".to_string(),
            "gwt-expired-deadline".to_string(),
            sentinel.to_string_lossy().into_owned(),
        ];
        let error = spawn_logged_with_deadline(
            &ProcessConsoleHub::new(),
            ProcessKind::Gh,
            "sh",
            &args,
            SpawnOptions::new("test expired deadline"),
            std::time::Instant::now() - Duration::from_millis(1),
        )
        .await
        .expect_err("expired deadline must fail before spawn");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(!sentinel.exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn deadline_terminates_and_reaps_child_process_tree() {
        let directory = tempfile::tempdir().expect("tempdir");
        let parent_file = directory.path().join("parent.pid");
        let descendant_file = directory.path().join("descendant.pid");
        let args = vec![
            "-c".to_string(),
            "echo $$ > \"$1\"; sleep 30 & echo $! > \"$2\"; wait".to_string(),
            "gwt-deadline-tree".to_string(),
            parent_file.to_string_lossy().into_owned(),
            descendant_file.to_string_lossy().into_owned(),
        ];
        let started = std::time::Instant::now();
        let error = spawn_logged_with_deadline(
            &ProcessConsoleHub::new(),
            ProcessKind::Gh,
            "sh",
            &args,
            SpawnOptions::new("test deadline tree"),
            started + Duration::from_millis(500),
        )
        .await
        .expect_err("long-running process tree must time out");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(3));

        let parent = read_pid(&parent_file);
        let descendant = read_pid(&descendant_file);
        wait_for_process_exit(parent);
        wait_for_process_exit(descendant);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn deadline_covers_descendant_held_output_pipe_without_reader_task_leak() {
        let directory = tempfile::tempdir().expect("tempdir");
        let descendant_file = directory.path().join("descendant.pid");
        let args = vec![
            "-c".to_string(),
            "sleep 30 & echo $! > \"$1\"; exit 0".to_string(),
            "gwt-deadline-pipe".to_string(),
            descendant_file.to_string_lossy().into_owned(),
        ];
        let hub = ProcessConsoleHub::new();
        let started = std::time::Instant::now();
        let error = spawn_logged_with_deadline(
            &hub,
            ProcessKind::Gh,
            "sh",
            &args,
            SpawnOptions::new("test descendant pipe"),
            started + Duration::from_millis(500),
        )
        .await
        .expect_err("descendant-held pipe must share the deadline");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(3));
        wait_for_process_exit(read_pid(&descendant_file));
        let line_count = hub.snapshot_kind(ProcessKind::Gh).len();
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert_eq!(hub.snapshot_kind(ProcessKind::Gh).len(), line_count);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn deadline_terminates_and_reaps_child_process_tree_windows() {
        // T-IDX-418 (SPEC #1939 Phase 70d): Windows counterpart of the POSIX
        // deadline tree test — the descendant a child backgrounds must not
        // survive the deadline-driven Job Object close.
        let directory = tempfile::tempdir().expect("tempdir");
        let descendant_file = directory.path().join("descendant.pid");
        let script = format!(
            "$child = Start-Process ping -ArgumentList '-n','60','127.0.0.1' \
             -PassThru -WindowStyle Hidden; \
             Set-Content -Path '{}' -Value $child.Id -Encoding ascii; \
             Start-Sleep -Seconds 60",
            descendant_file.display()
        );
        let args = vec!["-NoProfile".to_string(), "-Command".to_string(), script];
        let started = std::time::Instant::now();
        let error = spawn_logged_with_deadline(
            &ProcessConsoleHub::new(),
            ProcessKind::Gh,
            "powershell",
            &args,
            SpawnOptions::new("test deadline tree windows").current_dir(directory.path()),
            started + WINDOWS_PROCESS_TREE_FIXTURE_BUDGET,
        )
        .await
        .expect_err("long-running windows process tree must time out");
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < WINDOWS_PROCESS_TREE_FIXTURE_BOUND);

        let descendant = wait_for_pid_file_windows(&descendant_file);
        wait_for_process_exit_windows(descendant);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn deadline_reaps_windows_descendant_after_root_exits_first_with_pipe_open() {
        let directory = tempfile::tempdir().expect("tempdir");
        let descendant_file = directory.path().join("descendant-pipe.pid");
        let script = format!(
            "$child = Start-Process powershell -ArgumentList '-NoProfile','-Command',\
             'Start-Sleep -Seconds 60' -PassThru -NoNewWindow; \
             Set-Content -Path '{}' -Value $child.Id -Encoding ascii; exit 0",
            descendant_file.display()
        );
        let args = vec!["-NoProfile".to_string(), "-Command".to_string(), script];
        let started = Instant::now();
        let error = spawn_logged_with_deadline(
            &ProcessConsoleHub::new(),
            ProcessKind::IndexRunner,
            "powershell",
            &args,
            SpawnOptions::new("windows root exits before pipe descendant")
                .forward_output(false)
                .current_dir(directory.path()),
            started + WINDOWS_PROCESS_TREE_FIXTURE_BUDGET,
        )
        .await
        .expect_err("descendant-held pipe must keep collection pending until deadline");

        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < WINDOWS_PROCESS_TREE_FIXTURE_BOUND);
        let descendant = wait_for_pid_file_windows(&descendant_file);
        wait_for_process_exit_windows(descendant);
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn successful_deadline_spawn_releases_job_without_killing_detached_descendant() {
        let directory = tempfile::tempdir().expect("tempdir");
        let descendant_file = directory.path().join("detached.pid");
        let stdout_file = directory.path().join("detached.stdout");
        let stderr_file = directory.path().join("detached.stderr");
        let script = format!(
            "$child = Start-Process ping -ArgumentList '-n','60','127.0.0.1' \
             -PassThru -WindowStyle Hidden -RedirectStandardOutput '{}' \
             -RedirectStandardError '{}'; \
             Set-Content -Path '{}' -Value $child.Id -Encoding ascii; exit 0",
            stdout_file.display(),
            stderr_file.display(),
            descendant_file.display(),
        );
        let args = vec!["-NoProfile".to_string(), "-Command".to_string(), script];

        let output = spawn_logged_with_deadline(
            &ProcessConsoleHub::new(),
            ProcessKind::IndexRunner,
            "powershell",
            &args,
            SpawnOptions::new("windows detached success")
                .capture(false, false)
                .forward_output(false)
                .current_dir(directory.path()),
            Instant::now() + Duration::from_secs(10),
        )
        .await
        .expect("root process succeeds before deadline");
        assert!(output.success());

        let descendant = wait_for_pid_file_windows(&descendant_file);
        assert!(
            process_is_alive_windows(descendant),
            "successful Job release must preserve detached descendants"
        );
        let _ = crate::process::hidden_command("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Stop-Process -Id {descendant} -Force -ErrorAction SilentlyContinue"),
            ])
            .status();
        wait_for_process_exit_windows(descendant);
    }

    /// Absolute budget for the Windows process-tree fixtures.
    ///
    /// These fixtures must let PowerShell reach the statement that records the
    /// descendant pid before the deadline reaps the Job, otherwise the test
    /// cannot observe the tree it asserts on. Warm `powershell -NoProfile`
    /// plus one `Start-Process` measured 1.66s median / 2.05s p95 / 2.10s max
    /// under 24-way parallelism on the reference machine, so the previous 2s
    /// budget sat on the p95 and flaked whenever the whole crate ran at once.
    /// 15s keeps roughly a 7x margin for slower CI hosts; the fixtures run
    /// concurrently with the rest of the suite, so the wall-clock cost is paid
    /// once rather than once per fixture.
    #[cfg(windows)]
    const WINDOWS_PROCESS_TREE_FIXTURE_BUDGET: Duration = Duration::from_secs(15);

    /// Upper bound proving the deadline fired instead of the fixture running to
    /// completion. Kept well above the budget so it never turns into a second,
    /// tighter timing assertion.
    #[cfg(windows)]
    const WINDOWS_PROCESS_TREE_FIXTURE_BOUND: Duration = Duration::from_secs(60);

    #[cfg(windows)]
    fn wait_for_pid_file_windows(path: &std::path::Path) -> u32 {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if let Some(pid) = std::fs::read_to_string(path)
                .ok()
                .and_then(|raw| raw.trim().parse::<u32>().ok())
            {
                return pid;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!(
            "descendant pid file was not written at {} - the fixture process              never reached its pid-recording statement",
            path.display()
        );
    }

    #[cfg(windows)]
    fn wait_for_process_exit_windows(pid: u32) {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let filter = format!("PID eq {pid}");
        while std::time::Instant::now() < deadline {
            let output = crate::process::hidden_command("tasklist")
                .args(["/FI", filter.as_str(), "/NH"])
                .output()
                .expect("probe process via tasklist");
            let listing = String::from_utf8_lossy(&output.stdout);
            if !listing.contains(&pid.to_string()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("process {pid} remained alive after deadline cleanup");
    }

    #[cfg(windows)]
    fn process_is_alive_windows(pid: u32) -> bool {
        let filter = format!("PID eq {pid}");
        let output = crate::process::hidden_command("tasklist")
            .args(["/FI", filter.as_str(), "/NH"])
            .output()
            .expect("probe process via tasklist");
        String::from_utf8_lossy(&output.stdout).contains(&pid.to_string())
    }

    #[cfg(unix)]
    fn read_pid(path: &std::path::Path) -> u32 {
        std::fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
            .trim()
            .parse()
            .expect("numeric pid")
    }

    #[cfg(unix)]
    fn process_is_alive(pid: u32) -> bool {
        crate::process::hidden_command("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    #[cfg(unix)]
    fn wait_for_process_exit(pid: u32) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while std::time::Instant::now() < deadline {
            let status = crate::process::hidden_command("kill")
                .args(["-0", &pid.to_string()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("probe process");
            if !status.success() {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("process {pid} remained alive after deadline cleanup");
    }
}
