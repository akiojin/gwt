//! Issue #4405: a `verify.run` workload must not run at the low priority its
//! launcher inherited from the agent launch policy (SPEC #1921 Phase 86).
//!
//! Each test lowers the priority of this test process, which is why they live
//! in their own integration binary: the change cannot leak into other tests.

use std::process::Stdio;

use gwt_core::process_tree::spawn_at_normal_priority;

#[cfg(windows)]
#[test]
fn child_of_a_below_normal_launcher_runs_at_normal_priority() {
    use gwt_core::process_tree::{
        process_priority_class, set_process_priority_class, ProcessPriorityClass,
    };

    let own = std::process::id();
    set_process_priority_class(own, ProcessPriorityClass::BelowNormal)
        .expect("lower this test process like an agent tree");

    let mut command = gwt_core::process::hidden_command("cmd");
    command
        .args(["/C", "pause"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut spawned = spawn_at_normal_priority(&mut command).expect("spawn child");
    let class = process_priority_class(spawned.child.id());
    let _ = spawned.child.kill();
    let _ = spawned.child.wait();
    set_process_priority_class(own, ProcessPriorityClass::Normal).expect("restore priority");

    assert_eq!(
        class.expect("query child priority class"),
        ProcessPriorityClass::Normal,
        "the child must not inherit BELOW_NORMAL: {:?}",
        spawned.priority
    );
    assert!(spawned.priority.restored, "{:?}", spawned.priority);
}

/// Unix: an unprivileged process can never lower its nice value, so the
/// child keeps the launcher's nice unless the host grants the right. Either
/// way the report must say what the child actually runs at, and name the
/// inherited launch policy when it could not be restored.
#[cfg(unix)]
#[test]
fn child_of_a_niced_launcher_reports_its_effective_nice() {
    // SAFETY: plain syscalls on this process with no memory preconditions.
    let raised = unsafe { libc::setpriority(libc::PRIO_PROCESS as _, 0, 5) };
    assert_eq!(raised, 0, "raising our own nice value is always permitted");

    let mut command = gwt_core::process::hidden_command("sleep");
    command.arg("5").stdout(Stdio::null()).stderr(Stdio::null());
    let mut spawned = spawn_at_normal_priority(&mut command).expect("spawn child");
    // SAFETY: plain syscall reading the child's nice value.
    let child_nice =
        unsafe { libc::getpriority(libc::PRIO_PROCESS as _, spawned.child.id() as libc::id_t) };
    let _ = spawned.child.kill();
    let _ = spawned.child.wait();

    let report = spawned.priority;
    assert!(
        report.detail.contains(&format!("nice {child_nice}")),
        "the report must state the child's effective nice {child_nice}: {report:?}"
    );
    if child_nice > 0 {
        assert!(!report.restored, "{report:?}");
        assert!(
            report.detail.contains("inherited"),
            "an unrestored nice must name the inherited launch policy: {report:?}"
        );
    } else {
        assert!(report.restored, "{report:?}");
    }
}
