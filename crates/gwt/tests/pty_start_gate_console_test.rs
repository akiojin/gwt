//! Issue #3631 — the PTY start gate must keep the launched agent inside the
//! pane's pseudoconsole.
//!
//! Windows links `gwt.exe` with `windows_subsystem = "windows"`. A GUI
//! subsystem image is never attached to a pseudoconsole, so a start gate
//! hosted by `gwt.exe` runs with no console and NULL std handles; the target
//! it releases therefore gets a brand new console instead of the pane's
//! ConPTY, which Windows 11 hands to the configured default terminal
//! (Windows Terminal). The pane stays blank while the agent TUI renders in an
//! external window.
//!
//! The pre-existing start-gate coverage only asserted that the released target
//! *ran* (via a file sentinel), which stayed green throughout that failure.
//! This test asserts the released target's output actually reaches the PTY
//! master. `windows_binary_subsystem_test.rs` guards the underlying
//! console-subsystem invariant.

use std::{
    collections::HashMap,
    io::Read,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, OnceLock},
    time::{Duration, Instant},
};

use gwt::pty_start_gate::{resolve_pty_start_gate_program, PTY_START_GATE_ARG};
use gwt_terminal::{pty::SpawnConfig, PtyHandle};

const PANE_MARKER: &str = "GWT3631_PANE_OUTPUT_OK";
/// `CSI 1 ; 1 R` — the cursor-position report a terminal owes the pseudoconsole.
const CURSOR_POSITION_REPORT: &[u8] = b"\x1b[1;1R";

fn pty_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Reproduce the production launch shape: on Windows the agent command is
/// normalized into the `cmd.exe /d /v:off /c %GWT_WINDOWS_CMD_WRAPPER_EXPRESSION%`
/// wrapper before it is handed to the start gate.
fn marker_target_config() -> SpawnConfig {
    #[cfg(windows)]
    let (command, args, env) = (
        "cmd.exe".to_string(),
        vec![
            "/d".to_string(),
            "/v:off".to_string(),
            "/c".to_string(),
            format!(
                "%{}%",
                gwt_core::process::WINDOWS_CMD_WRAPPER_EXPRESSION_ENV
            ),
        ],
        HashMap::from([(
            gwt_core::process::WINDOWS_CMD_WRAPPER_EXPRESSION_ENV.to_string(),
            format!("echo {PANE_MARKER}"),
        )]),
    );

    #[cfg(not(windows))]
    let (command, args, env) = (
        "/bin/sh".to_string(),
        vec!["-c".to_string(), format!("printf '%s\\n' {PANE_MARKER}")],
        HashMap::new(),
    );

    SpawnConfig {
        command,
        args,
        cols: 80,
        rows: 24,
        env,
        remove_env: Vec::new(),
        cwd: None,
    }
}

/// Drain the PTY master into a shared buffer until `needle` shows up.
///
/// The caller keeps polling instead of stopping at EOF so a target that exits
/// immediately after printing still gets its bytes compared.
fn drain_until_contains(handle: &PtyHandle, timeout: Duration, needle: &str) -> String {
    let mut reader = handle.reader().expect("PTY reader");
    let collected = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&collected);
    std::thread::spawn(move || {
        let mut buf = vec![0_u8; 4096];
        let mut output = Vec::new();
        while let Ok(read) = reader.read(&mut buf) {
            if read == 0 {
                break;
            }
            output.extend_from_slice(&buf[..read]);
            *sink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) =
                String::from_utf8_lossy(&output).into_owned();
        }
    });

    let deadline = Instant::now() + timeout;
    loop {
        let text = collected
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if text.contains(needle) || Instant::now() >= deadline {
            return text;
        }
        // A real terminal answers every cursor-position query the pseudoconsole
        // asks; repeating it keeps a console client from parking on a query
        // that arrived after the one-shot answer.
        let _ = handle.write_input(CURSOR_POSITION_REPORT);
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn released_start_gate_target_renders_into_the_pane_pty() {
    let _guard = pty_test_lock();
    let gate_program = resolve_pty_start_gate_program(Path::new(env!("CARGO_BIN_EXE_gwt")))
        .expect("resolve the PTY start-gate program");

    let pending = PtyHandle::spawn_pending(
        marker_target_config(),
        gate_program.clone(),
        vec![PTY_START_GATE_ARG.to_string()],
        "issue-3631-nonce",
    )
    .expect("spawn pending PTY");
    let handle = pending.release().expect("release pending PTY");

    let output = drain_until_contains(&handle, Duration::from_secs(30), PANE_MARKER);
    let _ = handle.kill();

    assert!(
        output.contains(PANE_MARKER),
        "the released start-gate target must write into the pane PTY, but the \
         master read back {output:?} (gate program: {})",
        gate_program.display()
    );
}
