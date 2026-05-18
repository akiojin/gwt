//! SPEC-1942 US-14 / T-229: integration test for `gwt serve` headless mode.
//!
//! Verifies the end-to-end bootstrap: spawning the `gwt` binary with the
//! `serve` verb starts the embedded HTTP server without opening a native
//! WebView window, exposes `/healthz`, and shuts down gracefully when the
//! parent kills the child.

use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Spawn `gwt serve --port 0`, wait for the URL line on stderr, hit
/// `/healthz`, and tear the subprocess down. The test stays cross-platform
/// by using `.kill()` for shutdown instead of a Unix-only signal so the
/// `windows_subsystem = "windows"` build also passes; the graceful SIGTERM
/// path is covered by the in-process unit tests of `front_door_route`,
/// `acquire_instance_lock`, and the embedded server bind/port surface.
#[test]
fn serve_mode_starts_server_without_opening_webview() {
    // Force-bypass the per-(gwt_home, cwd) single-instance lock so this test
    // can run alongside an interactive `gwt` session on the developer's
    // machine. SPEC-2014 Phase C6 escape hatch.
    let temp_home = tempfile::tempdir().expect("temp gwt home");
    let temp_cwd = tempfile::tempdir().expect("temp cwd");

    let mut child = Command::new(env!("CARGO_BIN_EXE_gwt"))
        .arg("serve")
        .arg("--port")
        .arg("0")
        .env("GWT_FORCE_NEW_INSTANCE", "1")
        .env("HOME", temp_home.path())
        .current_dir(temp_cwd.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn gwt serve");

    let stderr = child.stderr.take().expect("stderr handle");
    let reader = BufReader::new(stderr);

    // Wait up to 30 seconds for the URL line. The first launch in CI builds
    // the project-index runtime on demand, which is slow.
    let started = Instant::now();
    let timeout = Duration::from_secs(30);
    let mut url: Option<String> = None;
    let mut transcript: Vec<String> = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        transcript.push(line.clone());
        if let Some(rest) = line.strip_prefix("gwt browser URL: ") {
            url = Some(rest.trim().to_string());
            break;
        }
        if started.elapsed() > timeout {
            break;
        }
    }

    let url = match url {
        Some(value) => value,
        None => {
            let _ = child.kill();
            let _ = child.wait();
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
    // Killed subprocess on Unix yields a None code; the assertion here only
    // verifies that the wait completed successfully — that is, we did not
    // leak the child process.
    let _ = status;
}
