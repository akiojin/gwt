//! PTY output and status workers independent of the GUI event loop.
//!
//! The caller retains the pane and thread handles; workers report observations
//! through a sink without creating another runtime owner.

use std::{
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Instant,
};

use gwt_terminal::{Pane, PaneStatus};

use crate::WindowProcessStatus;

/// Bound each parse and GUI notification while allowing PTY bursts in one read.
pub const OUTPUT_BATCH_MAX_BYTES: usize = 64 * 1024;

/// Observations from one exact pane incarnation, before any GUI adaptation.
#[derive(Debug)]
pub enum PaneRuntimeEvent {
    Output {
        id: String,
        incarnation: u64,
        data: Vec<u8>,
        seq: u64,
        screen: Option<String>,
        screen_ansi: Option<Vec<u8>>,
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
    current: Arc<AtomicBool>,
    sink: impl Fn(PaneRuntimeEvent) + Send + 'static,
) -> JoinHandle<()> {
    // Reserve the reader before the status worker can observe an immediate
    // exit and release descriptors. This clone still drains after release.
    let reader = pane
        .lock()
        .map_err(|error| error.to_string())
        .and_then(|pane| pane.reader().map_err(|error| error.to_string()));
    let dispatch = tracing::dispatcher::get_default(Clone::clone);
    thread::spawn(move || {
        let _dispatch = tracing::dispatcher::set_default(&dispatch);
        let reader = match reader {
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
        // A read can release the three withheld bytes of a split CSI 6n
        // candidate. Keep the resulting public chunk within the existing bound.
        let mut buffer = [0u8; OUTPUT_BATCH_MAX_BYTES - 3];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => {
                    let tail = pane.lock().ok().and_then(|mut pane| {
                        let data = pane.finish_owner_output();
                        (!data.is_empty()).then(|| {
                            (
                                data,
                                pane.output_seq(),
                                pane.screen().contents(),
                                pane.screen().contents_formatted(),
                            )
                        })
                    });
                    if let Some((data, seq, screen, screen_ansi)) = tail {
                        sink(PaneRuntimeEvent::Output {
                            id: id.clone(),
                            incarnation,
                            data,
                            seq,
                            screen: Some(screen),
                            screen_ansi: Some(screen_ansi),
                        });
                    }
                    break;
                }
                Ok(read) => {
                    let mut chunk = buffer[..read].to_vec();
                    let lock_started = Instant::now();
                    let mut seq = 0;
                    let mut screen = None;
                    let mut screen_ansi = None;
                    let mut responses = None;
                    if let Ok(mut pane) = pane.lock() {
                        let lock_wait_us = lock_started.elapsed().as_micros() as u64;
                        let parse_started = Instant::now();
                        let output = pane.process_owner_output(&chunk);
                        chunk = output.data;
                        responses = Some((pane.shared_pty(), output.cursor_reports));
                        seq = pane.output_seq();
                        screen = Some(pane.screen().contents());
                        screen_ansi = Some(pane.screen().contents_formatted());
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
                    // The parser captured each cursor at the query's stream
                    // position. Never hold Pane while waiting for its writer.
                    if let Some((pty, reports)) = responses {
                        if !reports.is_empty() {
                            tracing::debug!(
                                target: "gwt_terminal_protocol",
                                stage = "owner_cursor_response",
                                window_id = id.as_str(),
                                incarnation,
                                seq,
                                report_count = reports.len(),
                                "answering terminal cursor query"
                            );
                        }
                        for report in reports {
                            if let Err(error) = pty.write_cursor_position_report(report, &current) {
                                if current.load(Ordering::Acquire) {
                                    sink(PaneRuntimeEvent::Status {
                                        id,
                                        incarnation,
                                        status: WindowProcessStatus::Error,
                                        detail: Some(error.to_string()),
                                        exit_confirmed: false,
                                    });
                                    return;
                                }
                            }
                        }
                    }
                    sink(PaneRuntimeEvent::Output {
                        id: id.clone(),
                        incarnation,
                        data: chunk,
                        seq,
                        screen,
                        screen_ansi,
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
                let status = pane
                    .check_status()
                    .cloned()
                    .map_err(|error| error.to_string())?;
                // Exit confirmation belongs to this status observation. A
                // second process probe could observe an exit after a Running
                // status was copied and publish contradictory evidence.
                Ok((status, pane.last_exit().is_some()))
            });

        match status {
            Ok((status, exit_confirmed)) => {
                let (status, detail) = runtime_status_from_pane_status(&status);
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
    thread::spawn(move || {
        let pty = pane
            .lock()
            .map_err(|error| error.to_string())
            .map(|pane| pane.shared_pty());
        // The OS wait owns neither the pane mutex nor the child-state mutex.
        // PTY input, kill, and nonblocking probes can proceed while idle.
        let status = pty
            .and_then(|pty| pty.wait().map_err(|error| error.to_string()))
            .and_then(|_| {
                let mut pane = pane.lock().map_err(|error| error.to_string())?;
                let status = pane
                    .check_status()
                    .cloned()
                    .map_err(|error| error.to_string())?;
                // Descendants can keep the slave open after direct-child exit.
                // Terminate them so the reserved reader can drain to EOF.
                let _ = pane.kill();
                Ok(status)
            });

        match status {
            Ok(status) => {
                let (status, detail) = runtime_status_from_pane_status(&status);
                sink(PaneRuntimeEvent::Status {
                    id,
                    incarnation,
                    status,
                    detail,
                    exit_confirmed: true,
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
        sync::{atomic::AtomicBool, mpsc, Arc, Mutex},
        time::Duration,
    };

    use gwt_terminal::Pane;

    use super::{spawn_output_thread, spawn_status_thread, PaneRuntimeEvent};

    #[test]
    fn status_worker_has_no_idle_polling_timer() {
        let source = include_str!("pane_runtime.rs");
        let worker = source
            .split("pub fn spawn_status_thread(")
            .nth(1)
            .unwrap()
            .split("fn runtime_status_from_pane_status(")
            .next()
            .unwrap();
        assert!(
            !worker.contains("thread::sleep"),
            "idle panes must block on child exit instead of waking a polling timer"
        );
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
                // The descendant keeps the slave open after the direct child
                // exits. Status must therefore advance independently of EOF,
                // then terminate the remaining group so output drains fully.
                vec![
                    "-c",
                    "(trap '' HUP; sleep 60) & printf pane-worker-output; read line; exit 7",
                ],
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
        let (sender, receiver) = mpsc::channel();
        let output_sender = sender.clone();
        let output = spawn_output_thread(
            "pane-1".into(),
            42,
            pane.clone(),
            Arc::new(AtomicBool::new(true)),
            move |event| {
                let _ = output_sender.send(("output", event));
            },
        );
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
                    screen_ansi,
                } => {
                    assert!(data.len() <= super::OUTPUT_BATCH_MAX_BYTES);
                    assert!(screen.is_some(), "the parser worker supplies the screen");
                    assert!(
                        screen_ansi.is_some(),
                        "the parser worker supplies the same-sequence ANSI screen"
                    );
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
        assert!(!bytes.windows(4).any(|window| window == b"\x1b[6n"));
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
