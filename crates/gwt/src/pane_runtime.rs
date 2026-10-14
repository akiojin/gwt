//! PTY output and status workers independent of the GUI event loop.
//!
//! The caller retains the pane and thread handles; workers report observations
//! through a sink without creating another runtime owner.

use std::{
    io::Read,
    sync::{Arc, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use gwt_terminal::{Pane, PaneStatus};

use crate::WindowProcessStatus;

/// Observations from one exact pane incarnation, before any GUI adaptation.
#[derive(Debug)]
pub enum PaneRuntimeEvent {
    Output {
        id: String,
        incarnation: u64,
        data: Vec<u8>,
        seq: u64,
    },
    Status {
        id: String,
        incarnation: u64,
        status: WindowProcessStatus,
        detail: Option<String>,
        exit_confirmed: bool,
    },
}

/// Parse PTY output and report each chunk with its snapshot stream position.
pub fn spawn_output_thread(
    id: String,
    incarnation: u64,
    pane: Arc<Mutex<Pane>>,
    sink: impl Fn(PaneRuntimeEvent) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || {
        let reader = match pane
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|pane| pane.reader().map_err(|error| error.to_string()))
        {
            Ok(reader) => reader,
            Err(error) => {
                sink(PaneRuntimeEvent::Status {
                    id,
                    incarnation,
                    status: WindowProcessStatus::Error,
                    detail: Some(error),
                    exit_confirmed: false,
                });
                return;
            }
        };

        let mut reader = reader;
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let chunk = buffer[..read].to_vec();
                    let lock_started = Instant::now();
                    let mut seq = 0;
                    if let Ok(mut pane) = pane.lock() {
                        let lock_wait_us = lock_started.elapsed().as_micros() as u64;
                        let parse_started = Instant::now();
                        pane.process_bytes(&chunk);
                        seq = pane.output_seq();
                        let parse_us = parse_started.elapsed().as_micros() as u64;
                        // Log only when the contention window is large enough
                        // to plausibly starve a concurrent `write_input`. The
                        // threshold keeps the log volume bounded during
                        // normal output bursts while still surfacing the
                        // lock-hold windows that matter for drop triage.
                        if lock_wait_us > 500 || parse_us > 500 {
                            tracing::debug!(
                                target: "gwt_input_trace",
                                stage = "reader_pane_lock",
                                window_id = %id,
                                lock_wait_us,
                                parse_us,
                                "reader thread held pane mutex (output parsing)"
                            );
                        }
                    }
                    sink(PaneRuntimeEvent::Output {
                        id: id.clone(),
                        incarnation,
                        data: chunk,
                        seq,
                    });
                }
                Err(error) => {
                    sink(PaneRuntimeEvent::Status {
                        id: id.clone(),
                        incarnation,
                        status: WindowProcessStatus::Error,
                        detail: Some(error.to_string()),
                        exit_confirmed: false,
                    });
                    return;
                }
            }
        }

        let status = pane
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|mut pane| {
                pane.check_status()
                    .cloned()
                    .map_err(|error| error.to_string())
            });

        match status {
            Ok(status) => {
                let (status, detail) = runtime_status_from_pane_status(&status);
                let exit_confirmed = pane
                    .lock()
                    .ok()
                    .and_then(|pane| pane.process_has_exited().ok())
                    .unwrap_or(false);
                sink(PaneRuntimeEvent::Status {
                    id,
                    incarnation,
                    status,
                    detail,
                    exit_confirmed,
                });
            }
            Err(error) => {
                sink(PaneRuntimeEvent::Status {
                    id,
                    incarnation,
                    status: WindowProcessStatus::Error,
                    detail: Some(error),
                    exit_confirmed: false,
                });
            }
        }
    })
}

/// Observe process exit independently from the output reader reaching EOF.
pub fn spawn_status_thread(
    id: String,
    incarnation: u64,
    pane: Arc<Mutex<Pane>>,
    sink: impl Fn(PaneRuntimeEvent) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(100));
        let status = pane
            .lock()
            .map_err(|error| error.to_string())
            .and_then(|mut pane| {
                pane.check_status()
                    .cloned()
                    .map_err(|error| error.to_string())
            });

        match status {
            Ok(PaneStatus::Running) => continue,
            Ok(status) => {
                if matches!(status, PaneStatus::Completed(_)) {
                    if let Ok(pane) = pane.lock() {
                        let _ = pane.kill();
                    }
                }
                let (status, detail) = runtime_status_from_pane_status(&status);
                let exit_confirmed = pane
                    .lock()
                    .ok()
                    .and_then(|pane| pane.process_has_exited().ok())
                    .unwrap_or(false);
                sink(PaneRuntimeEvent::Status {
                    id: id.clone(),
                    incarnation,
                    status,
                    detail,
                    exit_confirmed,
                });
                if exit_confirmed {
                    break;
                }
            }
            Err(error) => {
                sink(PaneRuntimeEvent::Status {
                    id,
                    incarnation,
                    status: WindowProcessStatus::Error,
                    detail: Some(error),
                    exit_confirmed: false,
                });
                break;
            }
        }
    })
}

fn runtime_status_from_pane_status(status: &PaneStatus) -> (WindowProcessStatus, Option<String>) {
    match status {
        PaneStatus::Running => (WindowProcessStatus::Running, None),
        PaneStatus::Completed(0) => (
            crate::window_state::window_state_from_pane_status(status),
            Some("Process exited".to_string()),
        ),
        PaneStatus::Completed(code) => (
            crate::window_state::window_state_from_pane_status(status),
            Some(format!("Process exited with status {code}")),
        ),
        PaneStatus::Error(message) => (
            crate::window_state::window_state_from_pane_status(status),
            Some(message.clone()),
        ),
    }
}
#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::{mpsc, Arc, Mutex},
        time::Duration,
    };

    use gwt_terminal::Pane;

    use super::{spawn_output_thread, spawn_status_thread, PaneRuntimeEvent};

    #[test]
    fn workers_deliver_output_and_confirmed_exit_without_a_gui() {
        let temp = tempfile::tempdir().unwrap();
        let (command, args) = if cfg!(windows) {
            (
                "cmd",
                vec![
                    "/D",
                    "/S",
                    "/C",
                    "echo pane-worker-output & set /p pane_worker_input= & exit /B 7",
                ],
            )
        } else {
            (
                "/bin/sh",
                vec!["-c", "printf pane-worker-output; read line; exit 7"],
            )
        };
        let pane = Arc::new(Mutex::new(
            Pane::new(
                "pane-1".into(),
                command.into(),
                args.into_iter().map(str::to_owned).collect(),
                80,
                24,
                HashMap::new(),
                Some(temp.path().to_owned()),
            )
            .unwrap(),
        ));
        if cfg!(windows) {
            let _ = pane.lock().unwrap().write_input(b"\x1b[1;1R");
        }
        let (sender, receiver) = mpsc::channel();
        let output_sender = sender.clone();
        let output = spawn_output_thread("pane-1".into(), 42, pane.clone(), move |event| {
            let _ = output_sender.send(("output", event));
        });
        let status = spawn_status_thread("pane-1".into(), 42, pane.clone(), move |event| {
            let _ = sender.send(("status", event));
        });
        let mut bytes = Vec::new();
        let mut sequences = Vec::new();
        let mut confirmed_exit = None;
        let mut identities = Vec::new();
        let mut exit_requested = false;
        // Keep the child alive until its output reader has demonstrably started.
        // Then drain both sinks to closure so the final sequence includes every chunk.
        while let Ok((source, event)) = receiver.recv_timeout(Duration::from_secs(5)) {
            match event {
                PaneRuntimeEvent::Output {
                    id,
                    incarnation,
                    data,
                    seq,
                } => {
                    identities.push((id, incarnation));
                    bytes.extend(data);
                    sequences.push(seq);
                    if !exit_requested
                        && String::from_utf8_lossy(&bytes).contains("pane-worker-output")
                    {
                        pane.lock().unwrap().write_input(b"\r").unwrap();
                        exit_requested = true;
                    }
                }
                PaneRuntimeEvent::Status {
                    id,
                    incarnation,
                    status,
                    detail,
                    exit_confirmed,
                } => {
                    identities.push((id, incarnation));
                    if source == "status" && exit_confirmed {
                        confirmed_exit = Some((status, detail));
                    }
                }
            }
        }
        let _ = pane.lock().unwrap().kill();
        output.join().unwrap();
        status.join().unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains("pane-worker-output"));
        assert!(identities
            .iter()
            .all(|(id, incarnation)| id == "pane-1" && *incarnation == 42));
        assert!(!sequences.is_empty());
        assert!(sequences.iter().all(|seq| *seq > 0));
        assert!(sequences.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            sequences.last().copied(),
            Some(pane.lock().unwrap().output_seq())
        );
        // PaneStatus preserves the existing failed/successful projection;
        // the exact OS exit code remains on the pane receipt.
        assert_eq!(pane.lock().unwrap().last_exit().unwrap().exit_code, 7);
        assert_eq!(
            confirmed_exit,
            Some((
                crate::WindowProcessStatus::Error,
                Some("Process exited with status 1".into())
            ))
        );
    }
}
