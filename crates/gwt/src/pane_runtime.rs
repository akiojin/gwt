//! PTY output and status workers independent of the GUI event loop.
//!
//! The caller retains the pane and thread handles; workers report observations
//! through a sink without creating another runtime owner.

use std::{
    io::Read,
    sync::{mpsc, Arc, Mutex},
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
        /// Screen captured at `seq`, without a GUI-thread pane lock.
        screen: String,
    },
    Status {
        id: String,
        incarnation: u64,
        status: WindowProcessStatus,
        detail: Option<String>,
        exit_confirmed: bool,
    },
}

const PTY_READ_BYTES: usize = 4096;
const PTY_BATCH_BYTES: usize = 64 * 1024;
const PTY_READ_QUEUE_CHUNKS: usize = PTY_BATCH_BYTES / PTY_READ_BYTES;

/// Combine only queued, unparsed bytes. Never merge parsed events: a snapshot
/// can fall between their stream positions (Issue #4095).
fn next_output_batch(receiver: &mpsc::Receiver<Vec<u8>>) -> Option<Vec<u8>> {
    let mut batch = receiver.recv().ok()?;
    while batch.len() + PTY_READ_BYTES <= PTY_BATCH_BYTES {
        let Ok(chunk) = receiver.try_recv() else {
            break;
        };
        batch.extend_from_slice(&chunk);
    }
    Some(batch)
}

/// Parse bounded PTY batches and report each with its snapshot stream position.
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

        // The reader can get ahead while parsing/classifying a burst, but its
        // queued bytes are bounded. Sparse output is delivered without a timer.
        let (sender, receiver) = mpsc::sync_channel(PTY_READ_QUEUE_CHUNKS);
        let reader = thread::spawn(move || -> std::io::Result<()> {
            let mut reader = reader;
            let mut buffer = [0u8; PTY_READ_BYTES];
            loop {
                let read = reader.read(&mut buffer)?;
                if read == 0 || sender.send(buffer[..read].to_vec()).is_err() {
                    return Ok(());
                }
            }
        });
        while let Some(chunk) = next_output_batch(&receiver) {
            let lock_started = Instant::now();
            let mut seq = 0;
            let mut screen = String::new();
            if let Ok(mut pane) = pane.lock() {
                let lock_wait_us = lock_started.elapsed().as_micros() as u64;
                let parse_started = Instant::now();
                pane.process_bytes(&chunk);
                seq = pane.output_seq();
                screen = pane.screen().contents();
                let parse_us = parse_started.elapsed().as_micros() as u64;
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
                screen,
            });
        }
        let reader_error = match reader.join() {
            Ok(Ok(())) => None,
            Ok(Err(error)) => Some(error.to_string()),
            Err(_) => Some("PTY reader thread panicked".to_string()),
        };
        if let Some(error) = reader_error {
            sink(PaneRuntimeEvent::Status {
                id,
                incarnation,
                status: WindowProcessStatus::Error,
                detail: Some(error),
                exit_confirmed: false,
            });
            return;
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
    fn output_batches_preserve_bytes_and_leave_the_next_batch_queued() {
        let (sender, receiver) = mpsc::channel();
        let expected: Vec<u8> = (0..17).flat_map(|index| vec![index as u8; 4096]).collect();
        for chunk in expected.chunks(4096) {
            sender.send(chunk.to_vec()).unwrap();
        }
        drop(sender);

        let first = super::next_output_batch(&receiver).expect("first batch");
        let second = super::next_output_batch(&receiver).expect("remaining batch");
        assert_eq!(first.len(), 64 * 1024);
        assert_eq!(second.len(), 4096);
        assert_eq!([first, second].concat(), expected);
        assert!(super::next_output_batch(&receiver).is_none());
    }

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
                    screen,
                } => {
                    assert!(!screen.is_empty());
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
