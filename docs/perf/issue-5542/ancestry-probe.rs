//! Finite paired ancestry comparison; no Session or hook capability is used.
//! Compile with the checkout's sysinfo/windows artifacts (see ancestry-measurement.md).
use std::time::Instant;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

#[path = "../../../crates/gwt/src/process/ancestry.rs"]
mod ancestry;

fn baseline(pid: u32, ancestor: u32) -> (bool, usize) {
    let mut system = System::new().unwrap();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let details = system.processes().len();
    let mut current = Pid::from_u32(pid);
    for _ in 0..64 {
        let Some(parent) = system.process(current).and_then(sysinfo::Process::parent) else { return (false, details); };
        if parent.as_u32() == ancestor { return (true, details); }
        if parent.as_u32() <= 1 { return (false, details); }
        current = parent;
    }
    (false, details)
}

fn main() {
    let pid = std::process::id();
    let mut system = System::new().unwrap();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true, ProcessRefreshKind::nothing());
    let parent = system.process(Pid::from_u32(pid)).unwrap().parent().unwrap().as_u32();
    let mut rows = Vec::new();
    for round in 0..24 {
        for offset in 0..2 {
            let old = (round + offset) % 2 == 0;
            let started = Instant::now();
            let (matches, details) = if old { baseline(pid, parent) } else { (ancestry::is_descendant_of(pid, parent).unwrap(), 0) };
            let elapsed = started.elapsed().as_secs_f64() * 1000.0;
            assert!(matches, "both implementations must prove the same live ancestry");
            rows.push(format!("{{\"round\":{round},\"mode\":\"{}\",\"elapsed_ms\":{elapsed},\"process_details\":{details},\"matched\":{matches}}}", if old {"baseline"} else {"parent-only"}));
        }
    }
    for row in rows { println!("{row}"); }
}
