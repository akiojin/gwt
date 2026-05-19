//! SPEC-1942 US-14 / T-229: integration test for `gwt serve` headless mode.
//!
//! Verifies the end-to-end bootstrap: spawning the `gwt` binary with the
//! `serve` verb starts the embedded HTTP server without opening a native
//! WebView window, exposes `/healthz`, and shuts down gracefully when the
//! parent kills the child.
//!
//! ## Why this test is `#[ignore]` by default
//!
//! - **Linux**: `tao 0.35` unconditionally calls `gtk::init()` when
//!   creating the EventLoop (`platform_impl/linux/event_loop.rs:217`), so
//!   headless mode still requires a display server on Linux. CI runners do
//!   not provide `DISPLAY`. Migration off tao on Linux is tracked as a
//!   SPEC-1942 follow-up.
//! - **All CI runners**: a cold `~/.gwt` triggers slow project-index /
//!   chroma initialization that easily exceeds reasonable test budgets
//!   (the bootstrap blocks before any stderr output appears, so a deadline
//!   alone cannot distinguish a stuck process from a slow one).
//!
//! Developers should run the test locally with a warm `~/.gwt`:
//!
//! ```bash
//! cargo test -p gwt --test serve_mode_test -- --ignored
//! ```
//!
//! Local verification by the author on macOS yields ~7s when the cache is
//! warm. The headless bootstrap building blocks (route detection, argv
//! parsing, bind/port surface, access log middleware, lock kind isolation,
//! signal backstop) are covered by fast in-process unit tests that run in
//! every CI job.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

/// Spawn `gwt serve --port 0`, wait for the URL line on stderr, hit
/// `/healthz`, and tear the subprocess down. The test stays cross-platform
/// by using `.kill()` for shutdown instead of a Unix-only signal so the
/// `windows_subsystem = "windows"` build also passes; the graceful SIGTERM
/// path is covered by the in-process unit tests of `front_door_route`,
/// `acquire_instance_lock`, and the embedded server bind/port surface.
#[test]
#[ignore = "Spawns the full gwt binary and requires a warm ~/.gwt; opt in with --ignored"]
fn serve_mode_starts_server_without_opening_webview() {
    // Force-bypass the per-(gwt_home, cwd) single-instance lock so this test
    // can run alongside an interactive `gwt` session on the developer's
    // machine. SPEC-2014 Phase C6 escape hatch.
    //
    // Intentionally **do not** override `HOME`: a fresh `gwt_home` triggers a
    // cold chroma index initialization that easily exceeds the 180s test
    // budget. The developer's real `~/.gwt` cache reuse keeps cold-start to
    // <30s in practice, and `GWT_FORCE_NEW_INSTANCE=1` plus the kind-aware
    // lock from SPEC-1942 FR-099 stop the test from clashing with an active
    // GUI session on the same machine.
    let temp_cwd = tempfile::tempdir().expect("temp cwd");

    let mut child = Command::new(env!("CARGO_BIN_EXE_gwt"))
        .arg("serve")
        .arg("--port")
        .arg("0")
        .env("GWT_FORCE_NEW_INSTANCE", "1")
        .current_dir(temp_cwd.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwt serve");

    let stderr = child.stderr.take().expect("stderr handle");

    // SPEC-1942 US-14 follow-up: the prior in-loop `started.elapsed() >
    // timeout` check was ineffective because `reader.lines()` blocks until
    // a newline arrives. Move the read into a dedicated thread and use an
    // `mpsc` channel so the parent can enforce a wall-clock deadline.
    let (tx, rx) = mpsc::channel::<String>();
    let reader_handle = thread::Builder::new()
        .name("gwt-serve-stderr-reader".to_string())
        .spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        })
        .expect("spawn stderr reader thread");

    // First-time launches build the project-index runtime on demand, which
    // is slow on CI runners (Linux + cold cache routinely exceeds 60s
    // before the URL line lands). Use a generous budget; the test still
    // exits early as soon as the URL appears.
    let started = Instant::now();
    let timeout = Duration::from_secs(180);
    let mut url: Option<String> = None;
    let mut transcript: Vec<String> = Vec::new();
    loop {
        let remaining = match timeout.checked_sub(started.elapsed()) {
            Some(remaining) if !remaining.is_zero() => remaining,
            _ => break,
        };
        match rx.recv_timeout(remaining.min(Duration::from_secs(1))) {
            Ok(line) => {
                transcript.push(line.clone());
                if let Some(rest) = line.strip_prefix("gwt browser URL: ") {
                    url = Some(rest.trim().to_string());
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // poll again, the outer deadline check will break the loop
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    let url = match url {
        Some(value) => value,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader_handle.join();
            panic!(
                "did not observe 'gwt browser URL:' on stderr within {timeout:?}. transcript:\n{}",
                transcript.join("\n")
            );
        }
    };
    assert!(
        url.starts_with("http://127.0.0.1:"),
        "default serve mode must bind to 127.0.0.1, got {url}",
    );

    let healthz = format!("{url}healthz");
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client");
    let response = client.get(&healthz).send().expect("healthz request");
    assert_eq!(response.status().as_u16(), 200, "/healthz must return 200");
    assert_eq!(response.text().expect("healthz body"), "ok");

    let _ = child.kill();
    let status = child.wait().expect("wait for child");
    let _ = reader_handle.join();
    // Killed subprocess on Unix yields a None code; the assertion here only
    // verifies that the wait completed successfully — that is, we did not
    // leak the child process.
    let _ = status;
}
