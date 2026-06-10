//! SPEC-2014 FR-139..142 — mirror Docker launch preparation output into the
//! launching agent window's terminal.
//!
//! Docker-runtime launches run preflight probes, `docker compose ps` /
//! `up -d` (including image builds), and exec probes before the PTY starts.
//! Those lines already reach the Process Console hub (Console docker tab /
//! Logs Process facet, SPEC-2809); this mirror additionally forwards the
//! docker-kind lines to the agent terminal so a long image build is visible
//! in the terminal itself instead of leaving it silent until PTY handoff.
//!
//! The hub is global, so lines from a concurrently-launching second Docker
//! window would interleave here. Docker launches are serialized per window
//! by the wizard flow in practice, and the prep window is short; per-window
//! spawn-id scoping is intentionally left out of this first slice.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;
use std::time::Duration;

use gwt_core::process_console::{ProcessConsoleHub, ProcessKind};
use tokio::sync::broadcast::error::TryRecvError;

use super::{AppEventProxy, UserEvent};

const DRAIN_INTERVAL: Duration = Duration::from_millis(50);

/// Guard that mirrors docker-kind Process Console lines into a sink while a
/// Docker launch is preparing. Dropping (or calling [`Self::stop`]) performs
/// one final drain so lines emitted just before completion are not lost.
pub(crate) struct DockerLaunchOutputMirror {
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl DockerLaunchOutputMirror {
    /// Mirror docker launch output for `window_id`, forwarding chunks as
    /// `UserEvent::LaunchTerminalOutput` so the main event loop broadcasts
    /// them as `TerminalOutput` to the window's terminal.
    pub(crate) fn start(proxy: AppEventProxy, window_id: String) -> Self {
        Self::start_with_hub(gwt_core::process_console::global(), move |data| {
            proxy.send(UserEvent::LaunchTerminalOutput {
                window_id: window_id.clone(),
                data,
            });
        })
    }

    pub(crate) fn start_with_hub(
        hub: ProcessConsoleHub,
        sink: impl Fn(Vec<u8>) + Send + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);
        let mut receiver = hub.subscribe();
        let handle = thread::spawn(move || loop {
            // Read the stop flag before draining so one final drain runs
            // after stop is requested, flushing lines pushed right before
            // the launch preparation finished.
            let stopping = stop_flag.load(Ordering::Relaxed);
            let mut chunk: Vec<u8> = Vec::new();
            loop {
                match receiver.try_recv() {
                    Ok(line) => {
                        if line.kind == ProcessKind::Docker {
                            chunk.extend_from_slice(line.message.as_bytes());
                            chunk.extend_from_slice(b"\r\n");
                        }
                    }
                    Err(TryRecvError::Lagged(_)) => continue,
                    Err(TryRecvError::Empty) | Err(TryRecvError::Closed) => break,
                }
            }
            if !chunk.is_empty() {
                sink(chunk);
            }
            if stopping {
                break;
            }
            thread::sleep(DRAIN_INTERVAL);
        });
        Self {
            stop,
            handle: Some(handle),
        }
    }

    fn finish(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for DockerLaunchOutputMirror {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use gwt_core::process_console::{ProcessLine, ProcessStream};

    use super::*;

    type CollectedBytes = Arc<Mutex<Vec<u8>>>;

    fn collected_sink() -> (CollectedBytes, impl Fn(Vec<u8>) + Send + 'static) {
        let collected = Arc::new(Mutex::new(Vec::new()));
        let writer = Arc::clone(&collected);
        (collected, move |data: Vec<u8>| {
            writer.lock().unwrap().extend_from_slice(&data);
        })
    }

    #[test]
    fn mirrors_docker_lines_to_sink_with_crlf_and_skips_other_kinds() {
        let hub = ProcessConsoleHub::with_capacity(64);
        let (collected, sink) = collected_sink();
        let mirror = DockerLaunchOutputMirror::start_with_hub(hub.clone(), sink);

        hub.push(ProcessLine::new(
            ProcessKind::Docker,
            1,
            ProcessStream::Stderr,
            "#5 [app 1/3] FROM docker.io/library/ubuntu",
        ));
        hub.push(ProcessLine::new(
            ProcessKind::Gh,
            2,
            ProcessStream::Stdout,
            "gh noise line",
        ));
        drop(mirror);

        let output = String::from_utf8(collected.lock().unwrap().clone()).expect("utf8");
        assert!(
            output.contains("#5 [app 1/3] FROM docker.io/library/ubuntu\r\n"),
            "docker line missing: {output:?}"
        );
        assert!(
            !output.contains("gh noise line"),
            "non-docker line leaked: {output:?}"
        );
    }

    #[test]
    fn drains_pending_lines_on_immediate_stop() {
        let hub = ProcessConsoleHub::with_capacity(64);
        let (collected, sink) = collected_sink();
        let mirror = DockerLaunchOutputMirror::start_with_hub(hub.clone(), sink);

        for index in 0..10 {
            hub.push(ProcessLine::new(
                ProcessKind::Docker,
                1,
                ProcessStream::Stdout,
                format!("build step {index}"),
            ));
        }
        drop(mirror);

        let output = String::from_utf8(collected.lock().unwrap().clone()).expect("utf8");
        for index in 0..10 {
            assert!(
                output.contains(&format!("build step {index}\r\n")),
                "missing line {index}: {output:?}"
            );
        }
    }
}
