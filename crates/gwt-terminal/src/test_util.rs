//! Shared test utilities for gwt-terminal.

use std::{
    io::Read,
    sync::{mpsc, Mutex, MutexGuard, OnceLock},
    time::Duration,
};

static PTY_TEST_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

pub fn lock_pty_test() -> MutexGuard<'static, ()> {
    PTY_TEST_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("pty test lock poisoned")
}

/// Read from a PTY reader in a separate thread with timeout.
///
/// Returns accumulated output bytes, or an error message on failure/timeout.
pub fn read_with_timeout(
    mut reader: Box<dyn Read + Send>,
    timeout: Duration,
) -> Result<Vec<u8>, String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 4096];
        let mut output = Vec::new();
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    output.extend_from_slice(&buf[..n]);
                    let _ = tx.send(Ok(output.clone()));
                }
                Err(e) => {
                    let _ = tx.send(Err(e.to_string()));
                    break;
                }
            }
        }
    });

    let mut last_output = Vec::new();
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(data)) => last_output = data,
            Ok(Err(e)) => return Err(e),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !last_output.is_empty() {
                    return Ok(last_output);
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    if last_output.is_empty() {
        Err("Timed out with no output".to_string())
    } else {
        Ok(last_output)
    }
}
