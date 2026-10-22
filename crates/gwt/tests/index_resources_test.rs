//! Issue #4527 AC-1 / SPEC #1939 AS-33: the logical runner tree is measured
//! from the OS, not from `ps`, so Windows reports the same fields as POSIX.

use std::process::{Child, Stdio};

use gwt::index_resources::measure_process_tree;

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn spawn_idle_child() -> KillOnDrop {
    #[cfg(windows)]
    let mut command = {
        let mut command = gwt_core::process::hidden_command("cmd");
        command.args(["/C", "ping -n 30 127.0.0.1 > NUL"]);
        command
    };
    #[cfg(not(windows))]
    let mut command = {
        let mut command = gwt_core::process::hidden_command("sh");
        command.args(["-c", "sleep 30"]);
        command
    };
    KillOnDrop(
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn idle child"),
    )
}

#[test]
fn process_tree_usage_is_measured_for_the_whole_logical_tree() {
    let _child = spawn_idle_child();

    let usage = measure_process_tree(std::process::id()).expect("current process is measurable");

    assert!(
        usage.process_count >= 2,
        "the tree must include the spawned child: {usage:?}"
    );
    assert!(usage.rss_bytes > 0, "RSS must be measured: {usage:?}");
    assert!(
        usage.cpu_percent.is_finite() && usage.cpu_percent >= 0.0,
        "CPU must be a measured percentage: {usage:?}"
    );
    #[cfg(windows)]
    assert!(
        usage.private_bytes.is_some_and(|bytes| bytes > 0),
        "Windows must report private commit from the OS: {usage:?}"
    );
    #[cfg(not(windows))]
    assert_eq!(
        usage.private_bytes, None,
        "platforms without a private-commit counter report unsupported, never zero"
    );
}

#[test]
fn a_vanished_root_is_unmeasurable_rather_than_zero() {
    let mut child = spawn_idle_child();
    let pid = child.0.id();
    child.0.kill().expect("kill child");
    child.0.wait().expect("reap child");

    assert_eq!(measure_process_tree(pid), None);
}
