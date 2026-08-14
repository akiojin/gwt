//! Cross-platform PTY handle: spawn, I/O, resize, kill.

use std::{
    collections::{HashMap, VecDeque},
    io::{Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[cfg(not(windows))]
use std::path::Path;

use portable_pty::{native_pty_system, CommandBuilder, ExitStatus, MasterPty, PtySize};
use tracing::instrument;

use crate::TerminalError;

/// Phase C5 threshold (ms) above which a successful PTY resize is logged at
/// `warn` instead of `info`. Windows ConPTY's `ResizePseudoConsole` should
/// complete in single-digit milliseconds; anything north of 250 ms is a
/// strong signal of OS-level contention (Defender real-time scanning,
/// stalled child process, etc.) and worth surfacing in `~/.gwt/logs/`
/// without having to hand-correlate elapsed-time fields.
pub const SLOW_RESIZE_WARN_MS: u64 = 250;

mod process_group;
#[cfg(any(windows, test))]
mod windows_spawn;

use process_group::ProcessGroup;

/// Configuration for spawning a PTY process.
pub struct SpawnConfig {
    /// Command to execute.
    pub command: String,
    /// Command arguments.
    pub args: Vec<String>,
    /// Initial terminal size.
    pub cols: u16,
    /// Initial terminal rows.
    pub rows: u16,
    /// Environment variables to set.
    pub env: HashMap<String, String>,
    /// Inherited environment variable names to remove before applying `env`.
    pub remove_env: Vec<String>,
    /// Working directory.
    pub cwd: Option<PathBuf>,
}

const START_GATE_ENDPOINT_ENV: &str = "GWT_INTERNAL_PTY_GATE_ENDPOINT";
const START_GATE_NONCE_ENV: &str = "GWT_INTERNAL_PTY_GATE_NONCE";
const START_GATE_TARGET_ENV: &str = "GWT_INTERNAL_PTY_GATE_TARGET";
const START_GATE_HELLO: u8 = 1;
const START_GATE_RELEASE: u8 = 2;

/// A PTY child that has completed its private start-gate handshake but whose
/// real target has not begun executing.
pub struct PendingPty {
    handle: Option<PtyHandle>,
    gate: Option<TcpStream>,
}

impl PendingPty {
    /// Return the gate helper's process id. The helper preserves this identity
    /// when it replaces itself with the target on Unix.
    pub fn process_id(&self) -> Option<u32> {
        self.handle.as_ref().and_then(PtyHandle::process_id)
    }

    /// Release the helper to execute the target and return the live PTY.
    pub fn release(mut self) -> Result<PtyHandle, TerminalError> {
        let mut gate = self.gate.take().ok_or_else(|| TerminalError::PtyIoError {
            details: "PTY start gate is unavailable".to_string(),
        })?;
        if let Err(error) = gate
            .write_all(&[START_GATE_RELEASE])
            .and_then(|()| gate.flush())
        {
            let _ = self.abort_inner();
            return Err(TerminalError::PtyIoError {
                details: format!("release PTY start gate: {error}"),
            });
        }
        drop(gate);
        Ok(self.handle.take().expect("pending PTY owns its handle"))
    }

    /// Abort the pending launch without allowing the target to execute.
    pub fn abort(mut self) -> Result<(), TerminalError> {
        self.abort_inner()
    }

    fn abort_inner(&mut self) -> Result<(), TerminalError> {
        if let Some(gate) = self.gate.take() {
            let _ = gate.shutdown(Shutdown::Both);
        }
        self.handle.take().map_or(Ok(()), |handle| handle.kill())
    }
}

impl Drop for PendingPty {
    fn drop(&mut self) {
        let _ = self.abort_inner();
    }
}

struct SpawnedChildGuard {
    child: Option<Box<dyn portable_pty::Child + Send>>,
    process_group: Option<ProcessGroup>,
}

impl SpawnedChildGuard {
    fn new(child: Box<dyn portable_pty::Child + Send>) -> Self {
        Self {
            child: Some(child),
            process_group: None,
        }
    }

    fn terminate(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
        }
        if let Some(group) = self.process_group.as_mut() {
            group.terminate();
        }
    }

    fn into_parts(mut self) -> (Box<dyn portable_pty::Child + Send>, ProcessGroup) {
        (
            self.child.take().expect("spawn guard owns its child"),
            self.process_group.take().unwrap_or_default(),
        )
    }
}

impl Drop for SpawnedChildGuard {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum SpawnTestFailure {
    None,
    TakeWriter,
    ProcessGroupAttach,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SpawnDiagnostic {
    path_entry_count: usize,
    command_resolved_from_env_path: bool,
}

/// Handle to a spawned PTY process.
///
/// Provides methods for I/O, resize, and process lifecycle management.
/// Dropping a `PtyHandle` terminates the child and any descendants that were
/// attached to its process group / Job Object.
#[derive(Default)]
struct PtyInputState {
    protected: bool,
    queued: VecDeque<Vec<u8>>,
}

pub struct PtyHandle {
    master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send>>>,
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    input_state: Mutex<PtyInputState>,
    generation_active: AtomicBool,
    // Wrapped so `kill` (which takes `&self`) can synchronously terminate the
    // group without waiting for `Drop`. Declared last so that when `Drop` runs
    // the direct child has already been signaled above.
    process_group: Mutex<ProcessGroup>,
}

impl PtyHandle {
    /// Spawn a child process with a PTY.
    #[instrument(skip_all, fields(cmd = %config.command))]
    pub fn spawn(config: SpawnConfig) -> Result<Self, TerminalError> {
        Self::spawn_inner(config, SpawnTestFailure::None, None)
    }

    /// Spawn a trusted gate helper in the PTY and wait until it proves that it
    /// is blocking before returning. `gate_args_prefix` precedes the private
    /// environment-carried gate parameters, allowing binaries and test
    /// harnesses to select their hidden helper entrypoint.
    pub fn spawn_pending(
        config: SpawnConfig,
        gate_program: PathBuf,
        gate_args_prefix: Vec<String>,
        nonce: impl Into<String>,
    ) -> Result<PendingPty, TerminalError> {
        let config =
            normalize_spawn_config(config).map_err(|reason| TerminalError::PtyCreationFailed {
                reason: reason.to_string(),
            })?;
        let nonce = nonce.into();
        if nonce.is_empty() {
            return Err(TerminalError::PtyCreationFailed {
                reason: "PTY start-gate nonce must not be empty".to_string(),
            });
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|error| {
            TerminalError::PtyCreationFailed {
                reason: format!("bind PTY start gate: {error}"),
            }
        })?;
        listener
            .set_nonblocking(false)
            .map_err(|error| TerminalError::PtyCreationFailed {
                reason: format!("configure PTY start gate: {error}"),
            })?;
        let endpoint = listener
            .local_addr()
            .map_err(|error| TerminalError::PtyCreationFailed {
                reason: format!("resolve PTY start-gate endpoint: {error}"),
            })?;
        let target = encode_start_gate_target(&config.command, &config.args);
        let mut gate_config = SpawnConfig {
            command: gate_program.to_string_lossy().into_owned(),
            args: gate_args_prefix,
            cols: config.cols,
            rows: config.rows,
            env: config.env,
            remove_env: config.remove_env,
            cwd: config.cwd,
        };
        gate_config
            .env
            .insert(START_GATE_ENDPOINT_ENV.to_string(), endpoint.to_string());
        gate_config
            .env
            .insert(START_GATE_NONCE_ENV.to_string(), nonce.clone());
        gate_config
            .env
            .insert(START_GATE_TARGET_ENV.to_string(), target);

        let handle = Self::spawn(gate_config)?;
        if handle.process_id().is_none() {
            return Err(pending_spawn_error(
                handle,
                "PTY start-gate helper has no process id".to_string(),
            ));
        }
        if let Err(error) = listener.set_nonblocking(true) {
            return Err(pending_spawn_error(
                handle,
                format!("configure accept: {error}"),
            ));
        }
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut handle = Some(handle);
        loop {
            match listener.accept() {
                Ok((mut gate, peer)) if peer.ip().is_loopback() => {
                    gate.set_read_timeout(Some(Duration::from_secs(2)))
                        .map_err(|error| {
                            pending_spawn_error(
                                handle.take().expect("pending handle"),
                                format!("configure handshake: {error}"),
                            )
                        })?;
                    let mut hello = vec![0_u8; 1 + nonce.len()];
                    if let Err(error) = gate.read_exact(&mut hello) {
                        return Err(pending_spawn_error(
                            handle.take().expect("pending handle"),
                            format!("read PTY start-gate handshake: {error}"),
                        ));
                    }
                    if hello[0] != START_GATE_HELLO || hello[1..] != *nonce.as_bytes() {
                        return Err(pending_spawn_error(
                            handle.take().expect("pending handle"),
                            "PTY start-gate nonce mismatch".to_string(),
                        ));
                    }
                    gate.set_read_timeout(None).map_err(|error| {
                        pending_spawn_error(
                            handle.take().expect("pending handle"),
                            format!("finalize handshake: {error}"),
                        )
                    })?;
                    return Ok(PendingPty {
                        handle,
                        gate: Some(gate),
                    });
                }
                Ok((_gate, _)) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if Instant::now() >= deadline {
                        return Err(pending_spawn_error(
                            handle.take().expect("pending handle"),
                            "PTY start-gate handshake timed out".to_string(),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) => {
                    return Err(pending_spawn_error(
                        handle.take().expect("pending handle"),
                        format!("accept PTY start-gate handshake: {error}"),
                    ));
                }
            }
        }
    }

    fn spawn_inner(
        config: SpawnConfig,
        failure: SpawnTestFailure,
        observe_pid: Option<&mut dyn FnMut(u32)>,
    ) -> Result<Self, TerminalError> {
        let config =
            normalize_spawn_config(config).map_err(|reason| TerminalError::PtyCreationFailed {
                reason: reason.to_string(),
            })?;
        let pty_system = native_pty_system();
        let pair = pty_system
            .openpty(PtySize {
                rows: config.rows,
                cols: config.cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| TerminalError::PtyCreationFailed {
                reason: e.to_string(),
            })?;

        let mut cmd = CommandBuilder::new(&config.command);
        cmd.args(&config.args);
        if let Some(ref cwd) = config.cwd {
            cmd.cwd(cwd);
        }
        for key in &config.remove_env {
            cmd.env_remove(key);
        }
        for (key, value) in &config.env {
            cmd.env(key, value);
        }

        let child = match pair.slave.spawn_command(cmd) {
            Ok(child) => child,
            Err(error) => {
                let diagnostic = spawn_diagnostic(&config);
                let cwd = config
                    .cwd
                    .as_ref()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "none".to_string());
                tracing::error!(
                    target: "gwt::pty",
                    command = %config.command,
                    cwd = %cwd,
                    path_entry_count = diagnostic.path_entry_count,
                    command_resolved_from_env_path = diagnostic.command_resolved_from_env_path,
                    env_path = %env_path_for_log(&config.env),
                    error = %error,
                    "PTY spawn command failed"
                );
                return Err(TerminalError::PtyCreationFailed {
                    reason: error.to_string(),
                });
            }
        };

        let mut child = SpawnedChildGuard::new(child);
        let child_pid = child.child.as_ref().and_then(|child| child.process_id());
        if let (Some(pid), Some(observe)) = (child_pid, observe_pid) {
            observe(pid);
        }
        if failure == SpawnTestFailure::TakeWriter {
            return Err(TerminalError::PtyCreationFailed {
                reason: "take_writer: injected test failure".to_string(),
            });
        }
        if failure == SpawnTestFailure::ProcessGroupAttach {
            return Err(TerminalError::PtyCreationFailed {
                reason: "process group attach: injected test failure".to_string(),
            });
        }
        let process_group = match child_pid {
            Some(pid) => Some(ProcessGroup::attach(pid).map_err(|reason| {
                child.terminate();
                TerminalError::PtyCreationFailed {
                    reason: format!("process group attach: {reason}"),
                }
            })?),
            None => None,
        };
        child.process_group = process_group;
        let writer = pair.master.take_writer().map_err(|e| {
            child.terminate();
            TerminalError::PtyCreationFailed {
                reason: format!("take_writer: {e}"),
            }
        })?;
        let (child, process_group) = child.into_parts();

        Ok(Self {
            master: Arc::new(Mutex::new(pair.master)),
            child: Arc::new(Mutex::new(child)),
            writer: Arc::new(Mutex::new(writer)),
            input_state: Mutex::new(PtyInputState::default()),
            generation_active: AtomicBool::new(true),
            process_group: Mutex::new(process_group),
        })
    }

    #[cfg(test)]
    fn spawn_with_test_failure(
        config: SpawnConfig,
        failure: SpawnTestFailure,
        mut observe_pid: impl FnMut(u32),
    ) -> Result<Self, TerminalError> {
        Self::spawn_inner(config, failure, Some(&mut observe_pid))
    }

    /// Send bytes to the PTY stdin.
    pub fn write_input(&self, data: &[u8]) -> Result<(), TerminalError> {
        let mut state = self
            .input_state
            .lock()
            .map_err(|error| TerminalError::PtyIoError {
                details: format!("input state lock poisoned: {error}"),
            })?;
        if !self.generation_active.load(Ordering::Acquire) {
            return Err(TerminalError::PtyIoError {
                details: "PTY input generation is no longer active".to_string(),
            });
        }
        if state.protected {
            state.queued.push_back(data.to_vec());
            return Ok(());
        }

        let lock_started = Instant::now();
        let mut writer = self.writer.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        let lock_wait_us = lock_started.elapsed().as_micros() as u64;
        if !self.generation_active.load(Ordering::Acquire) {
            return Err(TerminalError::PtyIoError {
                details: "PTY input generation is no longer active".to_string(),
            });
        }
        drop(state);

        Self::write_with_locked_writer(&mut writer, data, lock_wait_us)
    }

    /// Reserve pane-wide input ordering across a multi-write submit. Ordinary
    /// key input is queued until the reservation is released.
    pub fn reserve_input_transaction(
        self: &Arc<Self>,
    ) -> Result<PtyInputReservation, TerminalError> {
        let mut state = self
            .input_state
            .lock()
            .map_err(|error| TerminalError::PtyIoError {
                details: format!("input state lock poisoned: {error}"),
            })?;
        if !self.generation_active.load(Ordering::Acquire) {
            return Err(TerminalError::PtyIoError {
                details: "PTY input generation is no longer active".to_string(),
            });
        }
        if state.protected {
            return Err(TerminalError::PtyIoError {
                details: "another protected PTY input transaction is active".to_string(),
            });
        }
        state.protected = true;
        drop(state);
        Ok(PtyInputReservation {
            handle: Arc::clone(self),
        })
    }

    /// Invalidate this writer generation at the physical-write commit point.
    /// Once this method returns, no writer from this generation can begin a
    /// later PTY mutation.
    pub fn invalidate_input_generation(&self) {
        self.generation_active.store(false, Ordering::Release);
        let _writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
    }

    fn write_with_locked_writer(
        writer: &mut Box<dyn Write + Send>,
        data: &[u8],
        lock_wait_us: u64,
    ) -> Result<(), TerminalError> {
        let write_started = Instant::now();
        let write_result = writer.write_all(data);
        let write_us = write_started.elapsed().as_micros() as u64;
        write_result.map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })?;

        let flush_started = Instant::now();
        let flush_result = writer.flush();
        let flush_us = flush_started.elapsed().as_micros() as u64;

        tracing::debug!(
            target: "gwt_input_trace",
            stage = "pty_writer",
            lock_wait_us,
            write_us,
            flush_us,
            ok = flush_result.is_ok(),
            "PTY writer completed write_all + flush"
        );

        flush_result.map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })?;
        Ok(())
    }

    fn release_protected_input(&self, writer: &mut Box<dyn Write + Send>) {
        loop {
            let queued = match self.input_state.lock() {
                Ok(mut state)
                    if self.generation_active.load(Ordering::Acquire)
                        && !state.queued.is_empty() =>
                {
                    state.queued.drain(..).collect::<Vec<_>>()
                }
                Ok(mut state) => {
                    state.queued.clear();
                    state.protected = false;
                    return;
                }
                Err(_) => return,
            };
            for bytes in queued {
                if let Err(error) = Self::write_with_locked_writer(writer, &bytes, 0) {
                    tracing::warn!(%error, "queued PTY input failed after protected submit");
                }
            }
        }
    }

    /// Resize the PTY window.
    ///
    /// Emits an `info` event at `target = gwt::resize::pty` with the resolved
    /// dimensions and total wall time so SPEC-2014 Phase C diagnostics can
    /// pinpoint Windows ConPTY stalls from `~/.gwt/logs/` alone. The lock and
    /// the underlying `MasterPty::resize` are timed separately because the
    /// lock contention pattern differs on Windows vs Unix.
    ///
    /// Phase C5: when `total_elapsed_ms` exceeds [`SLOW_RESIZE_WARN_MS`] the
    /// `info` event is upgraded to a `warn` so slow-path operators (and
    /// `~/.gwt/logs/` greps) can flag Windows ConPTY hangs without having to
    /// hand-correlate the elapsed-time field.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), TerminalError> {
        let started = Instant::now();
        let master = self.master.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        let lock_elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let resize_started = Instant::now();
        let outcome = master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        });
        let resize_elapsed_ms =
            u64::try_from(resize_started.elapsed().as_millis()).unwrap_or(u64::MAX);
        let total_elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        match outcome {
            Ok(()) => {
                if total_elapsed_ms >= SLOW_RESIZE_WARN_MS {
                    tracing::warn!(
                        target: "gwt::resize::pty",
                        cols = cols,
                        rows = rows,
                        lock_elapsed_ms = lock_elapsed_ms,
                        resize_elapsed_ms = resize_elapsed_ms,
                        total_elapsed_ms = total_elapsed_ms,
                        outcome = "slow",
                        threshold_ms = SLOW_RESIZE_WARN_MS,
                        "PTY resize completed but exceeded slow-path threshold"
                    );
                } else {
                    tracing::info!(
                        target: "gwt::resize::pty",
                        cols = cols,
                        rows = rows,
                        lock_elapsed_ms = lock_elapsed_ms,
                        resize_elapsed_ms = resize_elapsed_ms,
                        total_elapsed_ms = total_elapsed_ms,
                        outcome = "ok",
                        "PTY resize completed"
                    );
                }
                Ok(())
            }
            Err(e) => {
                let details = e.to_string();
                tracing::warn!(
                    target: "gwt::resize::pty",
                    cols = cols,
                    rows = rows,
                    lock_elapsed_ms = lock_elapsed_ms,
                    resize_elapsed_ms = resize_elapsed_ms,
                    total_elapsed_ms = total_elapsed_ms,
                    outcome = "error",
                    error = %details,
                    "PTY resize failed"
                );
                Err(TerminalError::PtyIoError { details })
            }
        }
    }

    /// Terminate the child process and every descendant in its process group.
    ///
    /// Terminating the group is required so that grandchildren cannot keep
    /// the PTY slave open after the direct child exits. While the slave stays
    /// open the master reader does not observe EOF, which would otherwise
    /// strand the reader thread (and its `Arc<Mutex<Pane>>`) and prevent the
    /// Drop chain from running.
    pub fn kill(&self) -> Result<(), TerminalError> {
        let mut child = self.child.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        let kill_result = child.kill();
        drop(child);

        // Always sweep descendants, even if the direct kill failed: the group
        // terminate is idempotent and uses an independent kernel path.
        let mut group = match self.process_group.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        group.terminate();
        drop(group);

        kill_result.map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })
    }

    /// Returns the OS process id of the spawned child, if available.
    pub fn process_id(&self) -> Option<u32> {
        self.child.lock().ok().and_then(|c| c.process_id())
    }

    /// Returns a reader for the PTY output.
    ///
    /// The reader can be used in a separate thread/task to read output asynchronously.
    pub fn reader(&self) -> Result<Box<dyn Read + Send>, TerminalError> {
        let master = self.master.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        master
            .try_clone_reader()
            .map_err(|e| TerminalError::PtyIoError {
                details: e.to_string(),
            })
    }

    /// Try to wait for the child process without blocking.
    ///
    /// Returns `Some(ExitStatus)` if the child has exited, `None` if still running.
    pub fn try_wait(&self) -> Result<Option<ExitStatus>, TerminalError> {
        let mut child = self.child.lock().map_err(|e| TerminalError::PtyIoError {
            details: format!("lock poisoned: {e}"),
        })?;
        child.try_wait().map_err(|e| TerminalError::PtyIoError {
            details: e.to_string(),
        })
    }
}

fn pending_spawn_error(handle: PtyHandle, reason: String) -> TerminalError {
    let _ = handle.kill();
    TerminalError::PtyCreationFailed { reason }
}

/// Run the trusted side of a PTY start gate configured by
/// [`PtyHandle::spawn_pending`]. The helper connects and proves it is blocked,
/// then executes the target only after the owning process sends release.
/// Closing the connection before release exits successfully without starting
/// the target.
#[allow(
    clippy::disallowed_methods,
    reason = "the parent already normalized the target before encoding it; Unix must use CommandExt::exec to preserve the gated PID"
)]
pub fn run_start_gate_from_env() -> Result<i32, TerminalError> {
    let endpoint = std::env::var(START_GATE_ENDPOINT_ENV).map_err(|error| {
        TerminalError::PtyCreationFailed {
            reason: format!("missing PTY start-gate endpoint: {error}"),
        }
    })?;
    let nonce =
        std::env::var(START_GATE_NONCE_ENV).map_err(|error| TerminalError::PtyCreationFailed {
            reason: format!("missing PTY start-gate nonce: {error}"),
        })?;
    let target =
        std::env::var(START_GATE_TARGET_ENV).map_err(|error| TerminalError::PtyCreationFailed {
            reason: format!("missing PTY start-gate target: {error}"),
        })?;
    let (command, args) = decode_start_gate_target(&target)?;
    if command.is_empty() || nonce.is_empty() {
        return Err(TerminalError::PtyCreationFailed {
            reason: "PTY start-gate target and nonce must not be empty".to_string(),
        });
    }

    let mut gate =
        TcpStream::connect(&endpoint).map_err(|error| TerminalError::PtyCreationFailed {
            reason: format!("connect PTY start gate at {endpoint}: {error}"),
        })?;
    let mut hello = Vec::with_capacity(1 + nonce.len());
    hello.push(START_GATE_HELLO);
    hello.extend_from_slice(nonce.as_bytes());
    gate.write_all(&hello)
        .and_then(|()| gate.flush())
        .map_err(|error| TerminalError::PtyIoError {
            details: format!("send PTY start-gate handshake: {error}"),
        })?;
    let mut release = [0_u8; 1];
    match gate.read_exact(&mut release) {
        Ok(()) if release[0] == START_GATE_RELEASE => {}
        Ok(()) => {
            return Err(TerminalError::PtyIoError {
                details: "invalid PTY start-gate release".to_string(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(0),
        Err(error) => {
            return Err(TerminalError::PtyIoError {
                details: format!("wait for PTY start-gate release: {error}"),
            })
        }
    }
    drop(gate);

    let mut target = Command::new(command);
    target.args(args);
    for key in [
        START_GATE_ENDPOINT_ENV,
        START_GATE_NONCE_ENV,
        START_GATE_TARGET_ENV,
    ] {
        target.env_remove(key);
    }

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;

        let error = target.exec();
        Err(TerminalError::PtyCreationFailed {
            reason: format!("execute released PTY target: {error}"),
        })
    }
    #[cfg(windows)]
    {
        let status = target
            .status()
            .map_err(|error| TerminalError::PtyCreationFailed {
                reason: format!("execute released PTY target: {error}"),
            })?;
        Ok(status.code().unwrap_or(1))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let status = target
            .status()
            .map_err(|error| TerminalError::PtyCreationFailed {
                reason: format!("execute released PTY target: {error}"),
            })?;
        Ok(status.code().unwrap_or(1))
    }
}

fn encode_start_gate_target(command: &str, args: &[String]) -> String {
    let mut encoded = String::new();
    for component in std::iter::once(command).chain(args.iter().map(String::as_str)) {
        encoded.push_str(&component.len().to_string());
        encoded.push(':');
        encoded.push_str(component);
    }
    encoded
}

fn decode_start_gate_target(encoded: &str) -> Result<(String, Vec<String>), TerminalError> {
    let mut remaining = encoded;
    let mut components = Vec::new();
    while !remaining.is_empty() {
        let separator = remaining
            .find(':')
            .ok_or_else(|| TerminalError::PtyCreationFailed {
                reason: "invalid PTY start-gate target length".to_string(),
            })?;
        let length = remaining[..separator].parse::<usize>().map_err(|error| {
            TerminalError::PtyCreationFailed {
                reason: format!("invalid PTY start-gate target length: {error}"),
            }
        })?;
        remaining = &remaining[separator + 1..];
        if length > remaining.len() || !remaining.is_char_boundary(length) {
            return Err(TerminalError::PtyCreationFailed {
                reason: "invalid PTY start-gate target component".to_string(),
            });
        }
        components.push(remaining[..length].to_string());
        remaining = &remaining[length..];
    }
    let mut components = components.into_iter();
    let command = components
        .next()
        .ok_or_else(|| TerminalError::PtyCreationFailed {
            reason: "PTY start-gate target is empty".to_string(),
        })?;
    Ok((command, components.collect()))
}

/// Owned guard for a protected input sequence that may cross a worker delay.
pub struct PtyInputReservation {
    handle: Arc<PtyHandle>,
}

impl PtyInputReservation {
    pub fn write_input(&self, data: &[u8]) -> Result<(), TerminalError> {
        if !self.handle.generation_active.load(Ordering::Acquire) {
            return Err(TerminalError::PtyIoError {
                details: "PTY input generation is no longer active".to_string(),
            });
        }
        let mut writer = self
            .handle
            .writer
            .lock()
            .map_err(|error| TerminalError::PtyIoError {
                details: format!("lock poisoned: {error}"),
            })?;
        if !self.handle.generation_active.load(Ordering::Acquire) {
            return Err(TerminalError::PtyIoError {
                details: "PTY input generation is no longer active".to_string(),
            });
        }
        PtyHandle::write_with_locked_writer(&mut writer, data, 0)
    }

    /// Acquire the physical writer first, then let the caller linearize its
    /// revocable authorization around the actual write. This prevents a
    /// writer-lock stall from carrying an authority check past its deadline.
    pub fn write_input_authorized(
        &self,
        data: &[u8],
        authorize_and_commit: impl FnOnce(
            &mut dyn FnMut(&mut dyn FnMut()) -> Result<(), TerminalError>,
        ) -> Result<(), TerminalError>,
    ) -> Result<(), TerminalError> {
        let mut writer = self
            .handle
            .writer
            .lock()
            .map_err(|error| TerminalError::PtyIoError {
                details: format!("lock poisoned: {error}"),
            })?;
        if !self.handle.generation_active.load(Ordering::Acquire) {
            return Err(TerminalError::PtyIoError {
                details: "PTY input generation is no longer active".to_string(),
            });
        }
        let mut commit = |mark_attempted: &mut dyn FnMut()| {
            if !self.handle.generation_active.load(Ordering::Acquire) {
                return Err(TerminalError::PtyIoError {
                    details: "PTY input generation is no longer active".to_string(),
                });
            }
            mark_attempted();
            PtyHandle::write_with_locked_writer(&mut writer, data, 0)
        };
        authorize_and_commit(&mut commit)
    }
}

impl Drop for PtyInputReservation {
    fn drop(&mut self) {
        if let Ok(mut writer) = self.handle.writer.lock() {
            self.handle.release_protected_input(&mut writer);
        } else if let Ok(mut state) = self.handle.input_state.lock() {
            state.queued.clear();
            state.protected = false;
        }
    }
}

fn normalize_spawn_config(config: SpawnConfig) -> Result<SpawnConfig, String> {
    #[cfg(windows)]
    {
        windows_spawn::normalize_spawn_config(config).map_err(|failure| failure.to_string())
    }

    #[cfg(not(windows))]
    {
        Ok(normalize_non_windows_spawn_config(config))
    }
}

/// Pre-spawn guard shared by the direct PTY path and by host-shell launchers
/// (which wrap the resolved command into cmd/PowerShell). Returns `Some(reason)`
/// when `command` resolves to a Windows `.exe`/`.com` that is not a valid PE
/// image (a native-binary placeholder stub or a corrupt file) — the file
/// Windows would otherwise reject with the misleading "unsupported 16-bit
/// application" dialog. Returns `None` on non-Windows and for valid executables
/// or bare command names.
pub fn reject_non_pe_executable(command: &str) -> Option<String> {
    #[cfg(windows)]
    {
        windows_spawn::reject_non_pe_executable(command)
    }

    #[cfg(not(windows))]
    {
        let _ = command;
        None
    }
}

/// Resolve a Windows command exactly as the PTY spawn path would, without
/// applying PTY-specific shell wrappers. Host-shell launchers use this before
/// embedding the command into `cmd.exe` / PowerShell scripts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NormalizedWindowsHostShellCommand {
    /// Resolved native program consumed by the outer host shell.
    pub command: String,
    /// Resolver-owned prefix followed by the caller arguments.
    pub args: Vec<String>,
    /// Effective caller environment plus resolver-owned wrapper values.
    pub env: HashMap<String, String>,
}

pub fn normalize_command_for_windows_host_shell(
    command: &str,
    args: &[String],
    env: &HashMap<String, String>,
    remove_env: &[String],
) -> Result<NormalizedWindowsHostShellCommand, String> {
    #[cfg(windows)]
    {
        windows_spawn::normalize_host_shell_command(command, args, env, remove_env)
            .map_err(|failure| failure.to_string())
    }

    #[cfg(not(windows))]
    {
        let _ = remove_env;
        Ok(NormalizedWindowsHostShellCommand {
            command: command.to_string(),
            args: args.to_vec(),
            env: env.clone(),
        })
    }
}

fn spawn_diagnostic(config: &SpawnConfig) -> SpawnDiagnostic {
    SpawnDiagnostic {
        path_entry_count: env_path_value(&config.env)
            .map(|path| std::env::split_paths(path).count())
            .unwrap_or(0),
        command_resolved_from_env_path: command_resolves_from_env_path(config),
    }
}

#[cfg(not(windows))]
fn command_resolves_from_env_path(config: &SpawnConfig) -> bool {
    resolve_command_from_env_path(&config.command, &config.env).is_some()
}

#[cfg(windows)]
fn command_resolves_from_env_path(_config: &SpawnConfig) -> bool {
    false
}

fn env_path_value(env: &HashMap<String, String>) -> Option<&str> {
    env.get("PATH")
        .or_else(|| env.get("Path"))
        .or_else(|| env.get("path"))
        .map(String::as_str)
}

fn env_path_for_log(env: &HashMap<String, String>) -> &str {
    env_path_value(env).unwrap_or("<unset>")
}

#[cfg(not(windows))]
fn normalize_non_windows_spawn_config(mut config: SpawnConfig) -> SpawnConfig {
    if let Some(command) = resolve_command_from_env_path(&config.command, &config.env) {
        config.command = command.display().to_string();
    }
    config
}

#[cfg(not(windows))]
fn resolve_command_from_env_path(command: &str, env: &HashMap<String, String>) -> Option<PathBuf> {
    if command.is_empty() || command.contains('/') {
        return None;
    }
    let path_value = env.get("PATH")?;
    std::env::split_paths(path_value).find_map(|dir| {
        let candidate = dir.join(command);
        is_executable_file(&candidate).then_some(candidate)
    })
}

#[cfg(all(not(windows), unix))]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(all(not(windows), not(unix)))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

impl Drop for PtyHandle {
    fn drop(&mut self) {
        // Best-effort termination: must never panic from Drop and must not
        // block the caller for long. Tolerate poisoned mutexes.
        let mut guard = match self.child.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        let _ = guard.kill();

        // Short reap loop so subsequent try_wait callers observe the exit.
        // Capped at ~500ms so Drop never stalls the UI thread.
        for _ in 0..20 {
            match guard.try_wait() {
                Ok(Some(_)) | Err(_) => break,
                Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            }
        }
        drop(guard);

        // Belt-and-suspenders: explicitly terminate the group in case `kill`
        // was never called (e.g. the handle was dropped without going through
        // stop_window_runtime). ProcessGroup::terminate is idempotent.
        if let Ok(mut group) = self.process_group.lock() {
            group.terminate();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };

    use super::*;
    use crate::test_util::{
        answer_cursor_position_query, echo_command, env_command, lock_pty_test, pwd_command,
        read_until_contains, read_with_timeout, sleep_command, stdin_echo_command, success_command,
        TestCommand,
    };
    use tracing::{
        field::{Field, Visit},
        span::{Attributes, Id, Record},
        Event, Metadata, Subscriber,
    };

    #[derive(Debug, Clone)]
    struct CapturedInputTrace {
        target: String,
        fields: HashMap<String, String>,
    }

    #[derive(Clone)]
    struct CaptureInputTraceSubscriber {
        events: Arc<Mutex<Vec<CapturedInputTrace>>>,
    }

    impl Subscriber for CaptureInputTraceSubscriber {
        fn enabled(&self, _metadata: &Metadata<'_>) -> bool {
            true
        }

        fn new_span(&self, _span: &Attributes<'_>) -> Id {
            Id::from_u64(1)
        }

        fn record(&self, _span: &Id, _values: &Record<'_>) {}

        fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

        fn event(&self, event: &Event<'_>) {
            let mut visitor = CaptureInputTraceVisitor::default();
            event.record(&mut visitor);
            self.events
                .lock()
                .expect("captured input trace events")
                .push(CapturedInputTrace {
                    target: event.metadata().target().to_string(),
                    fields: visitor.fields,
                });
        }

        fn enter(&self, _span: &Id) {}

        fn exit(&self, _span: &Id) {}
    }

    #[derive(Default)]
    struct CaptureInputTraceVisitor {
        fields: HashMap<String, String>,
    }

    impl CaptureInputTraceVisitor {
        fn insert(&mut self, field: &Field, value: impl ToString) {
            self.fields
                .insert(field.name().to_string(), value.to_string());
        }
    }

    impl Visit for CaptureInputTraceVisitor {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.insert(field, format!("{value:?}").trim_matches('"'));
        }

        fn record_str(&mut self, field: &Field, value: &str) {
            self.insert(field, value);
        }

        fn record_u64(&mut self, field: &Field, value: u64) {
            self.insert(field, value);
        }

        fn record_bool(&mut self, field: &Field, value: bool) {
            self.insert(field, value);
        }
    }

    fn capture_input_traces(run: impl FnOnce()) -> Vec<CapturedInputTrace> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let subscriber = CaptureInputTraceSubscriber {
            events: Arc::clone(&events),
        };
        tracing::subscriber::with_default(subscriber, run);
        let captured = events.lock().expect("captured input trace events").clone();
        captured
    }

    fn command_config(command: TestCommand) -> SpawnConfig {
        SpawnConfig {
            command: command.command,
            args: command.args,
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        }
    }

    fn echo_config(msg: &str) -> SpawnConfig {
        command_config(echo_command(msg))
    }

    fn sleep_config(secs: &str) -> SpawnConfig {
        command_config(sleep_command(secs))
    }

    #[cfg(unix)]
    fn wait_for_process_exit(pid: u32) -> bool {
        for _ in 0..100 {
            if unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[cfg(unix)]
    #[test]
    fn take_writer_failure_reaps_the_spawned_child() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let _pty_guard = lock_pty_test();
        let observed_pid = Arc::new(AtomicU32::new(0));
        let pid = Arc::clone(&observed_pid);
        let error = match PtyHandle::spawn_with_test_failure(
            sleep_config("60"),
            SpawnTestFailure::TakeWriter,
            move |child_pid| pid.store(child_pid, Ordering::SeqCst),
        ) {
            Ok(_) => panic!("injected writer failure must fail spawn"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("take_writer"));

        let pid = observed_pid.load(Ordering::SeqCst);
        assert!(pid > 0, "spawned child pid was not observed");
        assert!(wait_for_process_exit(pid), "writer failure orphaned {pid}");
    }

    #[cfg(unix)]
    #[test]
    fn process_group_attach_failure_reaps_the_spawned_child() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let _pty_guard = lock_pty_test();
        let observed_pid = Arc::new(AtomicU32::new(0));
        let pid = Arc::clone(&observed_pid);
        let error = match PtyHandle::spawn_with_test_failure(
            sleep_config("60"),
            SpawnTestFailure::ProcessGroupAttach,
            move |child_pid| pid.store(child_pid, Ordering::SeqCst),
        ) {
            Ok(_) => panic!("injected attach failure must fail spawn"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("process group"));

        let pid = observed_pid.load(Ordering::SeqCst);
        assert!(pid > 0, "spawned child pid was not observed");
        assert!(wait_for_process_exit(pid), "attach failure orphaned {pid}");
    }

    #[test]
    fn pty_writer_trace_uses_exact_content_free_allowlist() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(command_config(stdin_echo_command())).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let synthetic_input =
            b"typed_text_SENTINEL credential_SENTINEL env_secret_SENTINEL data_base64_SENTINEL\n";

        let traces = capture_input_traces(|| {
            handle
                .write_input(synthetic_input)
                .expect("synthetic PTY input write");
        });
        let event = traces
            .iter()
            .find(|event| {
                event.target == "gwt_input_trace"
                    && event.fields.get("stage").map(String::as_str) == Some("pty_writer")
            })
            .expect("pty_writer input trace");
        let mut actual = event.fields.keys().map(String::as_str).collect::<Vec<_>>();
        actual.sort_unstable();
        assert_eq!(
            actual,
            [
                "flush_us",
                "lock_wait_us",
                "message",
                "ok",
                "stage",
                "write_us",
            ]
        );
        let rendered = format!("{:?}", event.fields);
        for forbidden in [
            "typed_text_SENTINEL",
            "credential_SENTINEL",
            "env_secret_SENTINEL",
            "data_base64_SENTINEL",
            "data_len",
            "text_len",
            "chunk_len",
            "error",
        ] {
            assert!(!rendered.contains(forbidden), "trace leaked {forbidden}");
        }
    }

    fn cwd_output_matches(text: &str, canonical_cwd: &str) -> bool {
        let normalized_text = text.replace('/', "\\").to_ascii_lowercase();
        let normalized_cwd = canonical_cwd.replace('/', "\\").to_ascii_lowercase();
        if normalized_text.contains(&normalized_cwd)
            || normalized_cwd.contains(normalized_text.trim())
        {
            return true;
        }

        #[cfg(windows)]
        {
            let components = std::path::Path::new(canonical_cwd)
                .components()
                .filter_map(|component| match component {
                    std::path::Component::Normal(value) => {
                        Some(value.to_string_lossy().to_ascii_lowercase())
                    }
                    _ => None,
                })
                .collect::<Vec<_>>();
            if components.len() >= 2 {
                let suffix_len = components.len().min(3);
                let suffix = components[components.len() - suffix_len..].join("\\");
                return normalized_text.contains(&suffix);
            }
        }

        false
    }

    #[cfg(unix)]
    #[test]
    fn normalize_spawn_config_resolves_command_from_config_path() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let runner = dir.path().join("gwt-test-runner");
        std::fs::write(&runner, "#!/bin/sh\nexit 0\n").expect("write runner");
        let mut permissions = std::fs::metadata(&runner)
            .expect("runner metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&runner, permissions).expect("chmod runner");

        let config = SpawnConfig {
            command: "gwt-test-runner".to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env: HashMap::from([("PATH".to_string(), dir.path().display().to_string())]),
            remove_env: Vec::new(),
            cwd: None,
        };

        let normalized = normalize_spawn_config(config).expect("normalize spawn config");

        assert_eq!(PathBuf::from(normalized.command), runner);
    }

    #[cfg(unix)]
    #[test]
    fn spawn_diagnostic_reports_config_path_command_resolution() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let runner = dir.path().join("gwt-test-runner");
        std::fs::write(&runner, "#!/bin/sh\nexit 0\n").expect("write runner");
        let mut permissions = std::fs::metadata(&runner)
            .expect("runner metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&runner, permissions).expect("chmod runner");

        let config = SpawnConfig {
            command: "gwt-test-runner".to_string(),
            args: Vec::new(),
            cols: 80,
            rows: 24,
            env: HashMap::from([("PATH".to_string(), dir.path().display().to_string())]),
            remove_env: Vec::new(),
            cwd: None,
        };

        let diagnostic = spawn_diagnostic(&config);

        assert_eq!(diagnostic.path_entry_count, 1);
        assert!(diagnostic.command_resolved_from_env_path);
    }

    #[test]
    fn test_spawn_and_read_output() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(echo_config("hello")).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(text.contains("hello"), "Expected 'hello' in: {text}");
    }

    #[test]
    fn test_write_input() {
        let _pty_guard = lock_pty_test();
        let config = command_config(stdin_echo_command());
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader failed");
        handle.write_input(b"test-input\n").expect("write failed");
        let output =
            read_until_contains(reader, Duration::from_secs(5), "test-input").expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("test-input"),
            "Expected 'test-input' in: {text}"
        );
    }

    #[test]
    fn test_resize() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(sleep_config("1")).expect("spawn failed");
        handle.resize(120, 48).expect("resize should succeed");
    }

    #[test]
    fn test_kill() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(sleep_config("60")).expect("spawn failed");
        handle.kill().expect("kill should succeed");

        let mut exited = false;
        for _ in 0..50 {
            if let Ok(Some(_)) = handle.try_wait() {
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(exited, "Process should have exited after kill");
    }

    #[test]
    fn test_try_wait_running() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(sleep_config("60")).expect("spawn failed");
        let result = handle.try_wait().expect("try_wait failed");
        assert!(result.is_none(), "Process should still be running");
        handle.kill().ok();
    }

    #[test]
    fn test_try_wait_completed() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(echo_config("done")).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let mut exited = false;
        for _ in 0..50 {
            if let Ok(Some(status)) = handle.try_wait() {
                assert!(status.success());
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(exited, "Process should have completed");
    }

    #[test]
    fn test_spawn_with_env() {
        let _pty_guard = lock_pty_test();
        let mut env = HashMap::new();
        env.insert("GWT_TEST_VAR".to_string(), "test_value".to_string());
        let command = env_command();
        let config = SpawnConfig {
            command: command.command,
            args: command.args,
            cols: 80,
            rows: 24,
            env,
            remove_env: Vec::new(),
            cwd: None,
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("GWT_TEST_VAR=test_value"),
            "Expected env var in: {text}"
        );
    }

    #[test]
    fn test_spawn_with_cwd() {
        let _pty_guard = lock_pty_test();
        let temp = std::env::temp_dir();
        let command = pwd_command();
        let config = SpawnConfig {
            command: command.command,
            args: command.args,
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: Some(temp.clone()),
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output).trim().to_string();
        // The output should be the canonical path of the temp dir.
        // On macOS, /tmp -> /private/tmp or /var -> /private/var.
        let canonical_temp = std::fs::canonicalize(&temp)
            .unwrap_or(temp)
            .display()
            .to_string();
        let canonical_temp = canonical_temp
            .strip_prefix(r"\\?\")
            .unwrap_or(&canonical_temp)
            .to_string();
        assert!(
            cwd_output_matches(&text, &canonical_temp),
            "Expected temp dir path in output.\n  output: {text}\n  expected: {canonical_temp}"
        );
    }

    #[test]
    fn test_spawn_invalid_command_fails() {
        let _pty_guard = lock_pty_test();
        let config = SpawnConfig {
            command: "/nonexistent/binary".to_string(),
            args: vec![],
            cols: 80,
            rows: 24,
            env: HashMap::new(),
            remove_env: Vec::new(),
            cwd: None,
        };
        let result = PtyHandle::spawn(config);
        assert!(result.is_err());
    }

    #[test]
    fn test_spawn_with_removed_inherited_env() {
        let _pty_guard = lock_pty_test();
        let mut env = HashMap::new();
        env.insert("GWT_REMOVE_CHECK".to_string(), "expected".to_string());
        let command = env_command();
        let config = SpawnConfig {
            command: command.command,
            args: command.args,
            cols: 80,
            rows: 24,
            env,
            remove_env: vec!["HOME".to_string()],
            cwd: None,
        };
        let handle = PtyHandle::spawn(config).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let reader = handle.reader().expect("reader failed");
        let output = read_with_timeout(reader, Duration::from_secs(5)).expect("read failed");
        let text = String::from_utf8_lossy(&output);
        assert!(
            text.contains("GWT_REMOVE_CHECK=expected"),
            "Expected env var in: {text}"
        );
        assert!(
            !text.lines().any(|line| line.starts_with("HOME=")),
            "Expected inherited HOME to be removed from: {text}"
        );
    }

    #[test]
    fn test_success_command_exits_zero() {
        let _pty_guard = lock_pty_test();
        let handle = PtyHandle::spawn(command_config(success_command())).expect("spawn failed");
        answer_cursor_position_query(&handle);
        let mut exited = false;
        for _ in 0..50 {
            if let Ok(Some(status)) = handle.try_wait() {
                assert!(status.success());
                exited = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        assert!(exited, "Process should have completed");
    }

    #[test]
    fn protected_input_reservation_queues_ordinary_input_until_submit_finishes() {
        let _pty_guard = lock_pty_test();
        let handle = Arc::new(PtyHandle::spawn(sleep_config("2")).expect("spawn sleeper"));
        let reservation = Arc::clone(&handle)
            .reserve_input_transaction()
            .expect("reserve protected input");

        reservation.write_input(b"protected body").expect("body");
        handle
            .write_input(b"user input")
            .expect("ordinary input queues behind the reservation");
        {
            let state = handle.input_state.lock().expect("input state");
            assert!(state.protected);
            assert_eq!(state.queued.len(), 1);
        }

        reservation.write_input(b"\r").expect("submit");
        drop(reservation);
        let state = handle.input_state.lock().expect("settled input state");
        assert!(!state.protected);
        assert!(state.queued.is_empty());
    }

    #[test]
    fn generation_invalidation_fences_reserved_submit_and_drops_queued_input() {
        let _pty_guard = lock_pty_test();
        let handle = Arc::new(PtyHandle::spawn(sleep_config("2")).expect("spawn sleeper"));
        let reservation = Arc::clone(&handle)
            .reserve_input_transaction()
            .expect("reserve protected input");
        reservation.write_input(b"body").expect("body");
        handle.write_input(b"queued input").expect("queue input");

        handle.invalidate_input_generation();

        assert!(reservation.write_input(b"\r").is_err());
        drop(reservation);
        assert!(handle.write_input(b"late input").is_err());
        let state = handle.input_state.lock().expect("settled input state");
        assert!(state.queued.is_empty());
        assert!(!state.protected);
    }

    #[test]
    fn generation_invalidation_waits_for_the_physical_write_commit_point() {
        let _pty_guard = lock_pty_test();
        let handle = Arc::new(PtyHandle::spawn(sleep_config("2")).expect("spawn sleeper"));
        let writer = handle.writer.lock().expect("hold physical writer");
        let (invalidated_tx, invalidated_rx) = std::sync::mpsc::channel();
        let invalidating_handle = Arc::clone(&handle);
        let invalidator = std::thread::spawn(move || {
            invalidating_handle.invalidate_input_generation();
            invalidated_tx.send(()).expect("publish invalidation");
        });

        assert!(
            invalidated_rx
                .recv_timeout(Duration::from_millis(100))
                .is_err(),
            "invalidation must wait behind an already-authorized physical write"
        );
        drop(writer);
        invalidated_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("invalidation completes after the physical write commit point");
        invalidator.join().expect("invalidation thread");

        assert!(handle.write_input(b"late input").is_err());
    }

    #[test]
    fn authorized_commit_rechecks_generation_after_authorization_work() {
        let _pty_guard = lock_pty_test();
        let handle = Arc::new(PtyHandle::spawn(sleep_config("2")).expect("spawn sleeper"));
        let reservation = Arc::clone(&handle)
            .reserve_input_transaction()
            .expect("reserve protected input");
        let invalidating_handle = Arc::clone(&handle);

        let error = reservation
            .write_input_authorized(b"late body", move |commit| {
                invalidating_handle
                    .generation_active
                    .store(false, Ordering::Release);
                let mut attempted = || {};
                commit(&mut attempted)
            })
            .expect_err("generation invalidated during authorization must fail closed");

        assert!(error.to_string().contains("generation is no longer active"));
    }
}
