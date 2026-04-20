//! Lifecycle tests: dropping a PtyHandle must kill the child process.

#![cfg(unix)]

use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

use gwt_terminal::{
    pty::{PtyHandle, SpawnConfig},
    Pane,
};

fn sleep_config(secs: &str) -> SpawnConfig {
    SpawnConfig {
        command: "/bin/sleep".to_string(),
        args: vec![secs.to_string()],
        cols: 80,
        rows: 24,
        env: HashMap::new(),
        remove_env: Vec::new(),
        cwd: None,
    }
}

fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if !is_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn is_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[test]
fn dropping_pty_handle_terminates_child() {
    let handle = PtyHandle::spawn(sleep_config("60")).expect("spawn failed");
    let pid = handle.process_id().expect("process_id unavailable");
    assert!(is_alive(pid), "Child {pid} should be alive before drop");

    drop(handle);

    assert!(
        wait_for_exit(pid, Duration::from_secs(5)),
        "Child {pid} should be terminated within 5s of dropping PtyHandle"
    );
}

#[test]
fn dropping_pane_terminates_child() {
    let pane = Pane::new(
        "lifecycle-pane".to_string(),
        "/bin/sleep".to_string(),
        vec!["60".to_string()],
        80,
        24,
        HashMap::new(),
        None,
    )
    .expect("pane creation failed");

    let pid = pane.pty().process_id().expect("process_id unavailable");
    assert!(is_alive(pid), "Child {pid} should be alive before drop");

    drop(pane);

    assert!(
        wait_for_exit(pid, Duration::from_secs(5)),
        "Child {pid} should be terminated within 5s of dropping Pane"
    );
}
