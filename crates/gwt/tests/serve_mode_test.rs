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

    // SPEC-1942 2026-05-20 Amendment: `gwt serve` now auto-opens the system
    // browser by default. Pass `--no-open` so the regression test does not
    // pop a browser window on the developer's machine.
    let mut child = Command::new(env!("CARGO_BIN_EXE_gwt"))
        .arg("serve")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
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

    // SPEC-2785 US-2 AS-2 / FR-F: `gwt serve` must announce the Ctrl-C
    // shutdown contract on stderr so headless operators know how to stop
    // the server gracefully. We drain stderr a little further to confirm
    // the line, bounded by a small follow-up budget so this test never
    // blocks indefinitely on a quiet stderr.
    let mut saw_ctrl_c_hint = false;
    let ctrl_c_deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < ctrl_c_deadline {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(line) => {
                transcript.push(line.clone());
                if line.contains("gwt serve: press Ctrl-C to stop") {
                    saw_ctrl_c_hint = true;
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    if !saw_ctrl_c_hint {
        let _ = child.kill();
        let _ = child.wait();
        let _ = reader_handle.join();
        panic!(
            "expected 'gwt serve: press Ctrl-C to stop' on stderr after URL line. transcript:\n{}",
            transcript.join("\n")
        );
    }

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

/// SPEC-2785 US-2 AS-3 / FR-F: `GWT_BROWSER_URL_FILE` is the canonical
/// non-stderr handoff channel for CI / Playwright. Setting it to a writable
/// path must persist the bound URL there so harnesses can consume the URL
/// without parsing stderr.
#[test]
#[ignore = "Spawns the full gwt binary and requires a warm ~/.gwt; opt in with --ignored"]
fn serve_mode_writes_browser_url_to_handoff_file_when_env_set() {
    let temp_cwd = tempfile::tempdir().expect("temp cwd");
    let handoff_dir = tempfile::tempdir().expect("handoff dir");
    let handoff_path = handoff_dir.path().join("gwt-browser-url.txt");

    let mut child = Command::new(env!("CARGO_BIN_EXE_gwt"))
        .arg("serve")
        .arg("--port")
        .arg("0")
        .arg("--no-open")
        .env("GWT_FORCE_NEW_INSTANCE", "1")
        .env("GWT_BROWSER_URL_FILE", &handoff_path)
        .current_dir(temp_cwd.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwt serve");

    let stderr = child.stderr.take().expect("stderr handle");
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

    // Wait for the URL line so we know the embedded server has started and
    // the handoff file has been written by the parent process.
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
            Err(mpsc::RecvTimeoutError::Timeout) => {}
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

    // The handoff file is written by the parent immediately after the
    // stderr announcement, but the two are independent syscalls; tolerate
    // a tiny race by polling for a short while before failing.
    let mut file_contents = String::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(text) = std::fs::read_to_string(&handoff_path) {
            if !text.trim().is_empty() {
                file_contents = text;
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }

    let _ = child.kill();
    let _ = child.wait();
    let _ = reader_handle.join();

    assert!(
        !file_contents.is_empty(),
        "GWT_BROWSER_URL_FILE must be populated; path={}",
        handoff_path.display()
    );
    assert_eq!(
        file_contents.trim(),
        url.trim(),
        "handoff file contents must match the stderr URL line",
    );
    assert!(
        file_contents.trim().starts_with("http://127.0.0.1:"),
        "handoff URL must point at the local embedded server, got {file_contents:?}"
    );
}
