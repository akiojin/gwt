//! Issue #4742: exercise the priority roundtrip from both launcher classes.
//! This binary owns its process priority so changes cannot affect other tests.
#![cfg(windows)]

use gwt_core::process_tree::{
    process_priority_class, set_process_priority_class, ProcessPriorityClass,
};

#[test]
fn windows_process_priority_class_roundtrips_on_a_live_child() {
    let own = std::process::id();
    let original = process_priority_class(own).expect("query original launcher class");
    let result = std::panic::catch_unwind(|| {
        for initial in [
            ProcessPriorityClass::Normal,
            ProcessPriorityClass::BelowNormal,
        ] {
            set_process_priority_class(own, initial).expect("set launcher class");
            assert_eq!(
                process_priority_class(own).expect("query launcher class"),
                initial
            );
            eprintln!("priority roundtrip from {initial:?}");
            roundtrip();
        }
    });
    set_process_priority_class(own, original).expect("restore original launcher class");
    assert_eq!(
        process_priority_class(own).expect("query restored class"),
        original
    );
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}

#[allow(
    clippy::disallowed_methods,
    reason = "the test needs a plain paused child whose priority class it can mutate"
)]
fn roundtrip() {
    use std::process::{Command, Stdio};

    let mut child = Command::new("cmd")
        .args(["/C", "pause"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn paused child");
    let pid = child.id();
    let result = std::panic::catch_unwind(|| {
        let initial = process_priority_class(pid).expect("query initial class");
        eprintln!("child inherited {initial:?}");
        set_process_priority_class(pid, ProcessPriorityClass::BelowNormal)
            .expect("lower child priority class");
        assert_eq!(
            process_priority_class(pid).expect("query lowered class"),
            ProcessPriorityClass::BelowNormal
        );
        set_process_priority_class(pid, ProcessPriorityClass::Idle).expect("idle class");
        assert_eq!(
            process_priority_class(pid).expect("query idle class"),
            ProcessPriorityClass::Idle
        );
    });
    let _ = child.kill();
    let _ = child.wait();
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
