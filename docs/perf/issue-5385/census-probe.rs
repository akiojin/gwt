//! Read-only live census probe; compile against this checkout's built gwt library.
use std::{path::Path, time::Instant};

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}

#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcess() -> *mut std::ffi::c_void;
    fn GetProcessTimes(
        process: *mut std::ffi::c_void,
        created: *mut FileTime,
        exited: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
}

fn cpu_seconds() -> f64 {
    let (mut created, mut exited, mut kernel, mut user) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    // SAFETY: pseudo-handle belongs to this process; all output pointers are valid.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    };
    assert_ne!(ok, 0, "GetProcessTimes");
    let ticks = |t: FileTime| (u64::from(t.high) << 32) | u64::from(t.low);
    (ticks(kernel) + ticks(user)) as f64 / 10_000_000.0
}

fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    assert!(
        args.len() >= 2,
        "usage: census-probe <sessions-dir> <project>..."
    );
    for pass in 1..=2 {
        for root in &args[1..] {
            let before = cpu_seconds();
            let start = Instant::now();
            let inventory =
                gwt::session_inventory::observe_sessions(Path::new(root), Path::new(&args[0]));
            println!("project pass={pass} root={root:?} wall_seconds={:.6} cpu_seconds={:.6} sessions={} uncertainties={}",
                start.elapsed().as_secs_f64(), cpu_seconds() - before, inventory.sessions.len(), inventory.uncertainties.len());
        }
        let before = cpu_seconds();
        let start = Instant::now();
        let inventory = gwt::session_inventory::observe_machine_sessions(Path::new(&args[0]));
        println!(
            "machine pass={pass} wall_seconds={:.6} cpu_seconds={:.6} sessions={} uncertainties={}",
            start.elapsed().as_secs_f64(),
            cpu_seconds() - before,
            inventory.sessions.len(),
            inventory.uncertainties.len()
        );
    }
}
