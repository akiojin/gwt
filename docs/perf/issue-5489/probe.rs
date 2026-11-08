//! Read-only comparison against the same live Session/runtime files.
#![allow(dead_code)]
use std::{collections::BTreeMap, path::Path, sync::Mutex, time::Instant};

pub use gwt::{cli, pm_registry, process};
#[path = "../../../target/issue-5489-probe/after.rs"]
mod after_inventory;
#[path = "../../../target/issue-5489-probe/before.rs"]
mod before_inventory;

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
    // SAFETY: the pseudo-handle is current-process and output pointers are valid.
    assert_ne!(
        unsafe {
            GetProcessTimes(
                GetCurrentProcess(),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        },
        0
    );
    let ticks = |t: FileTime| (u64::from(t.high) << 32) | u64::from(t.low);
    (ticks(kernel) + ticks(user)) as f64 / 10_000_000.0
}

static METRICS: Mutex<BTreeMap<&'static str, (u64, f64, f64)>> = Mutex::new(BTreeMap::new());
fn measured<T>(kind: &'static str, f: impl FnOnce() -> T) -> T {
    let cpu = cpu_seconds();
    let clock = Instant::now();
    let result = f();
    let elapsed = clock.elapsed().as_secs_f64();
    let used = cpu_seconds() - cpu;
    let mut metrics = METRICS.lock().unwrap();
    let row = metrics.entry(kind).or_default();
    row.0 += 1;
    row.1 += used;
    row.2 += elapsed;
    result
}
pub fn measure_runtime_load(path: &Path) -> std::io::Result<gwt_agent::SessionRuntimeState> {
    measured("runtime_sidecar_load", || {
        gwt_agent::SessionRuntimeState::load(path)
    })
}
pub fn measure_session_load(path: &Path) -> std::io::Result<gwt_agent::Session> {
    measured("session_ledger_load", || gwt_agent::Session::load(path))
}
pub fn measure_process_refresh(
    system: &mut sysinfo::System,
    processes: sysinfo::ProcessesToUpdate<'_>,
    remove: bool,
    kind: sysinfo::ProcessRefreshKind,
) -> usize {
    let scope = match &processes {
        sysinfo::ProcessesToUpdate::All => "os_process_snapshot_all",
        sysinfo::ProcessesToUpdate::Some(_) => "os_process_refresh_named",
    };
    measured(scope, || {
        measured("os_process_refresh", || {
            system.refresh_processes_specifics(processes, remove, kind)
        })
    })
}

fn run(mode: &str, pair: usize, sessions: &Path, roots: &[String]) {
    METRICS.lock().unwrap().clear();
    let cpu = cpu_seconds();
    let clock = Instant::now();
    let mut root_rows = Vec::new();
    let mut capture_cpu = 0.0;
    let mut capture_wall = 0.0;
    if mode == "before" {
        for root in roots {
            let root_cpu = cpu_seconds();
            let root_clock = Instant::now();
            let inventory = before_inventory::observe_sessions(Path::new(root), sessions);
            root_rows.push(serde_json::json!({"root": root,
                "cpu_seconds": cpu_seconds() - root_cpu,
                "wall_seconds": root_clock.elapsed().as_secs_f64(),
                "sessions": inventory.sessions.len(), "uncertainties": inventory.uncertainties.len()}));
        }
    } else {
        let capture_clock = Instant::now();
        let capture_start_cpu = cpu_seconds();
        let census = after_inventory::ProjectSessionCensus::capture(sessions);
        capture_cpu = cpu_seconds() - capture_start_cpu;
        capture_wall = capture_clock.elapsed().as_secs_f64();
        for root in roots {
            let root_cpu = cpu_seconds();
            let root_clock = Instant::now();
            let inventory = census.observe(Path::new(root));
            root_rows.push(serde_json::json!({"root": root,
                "cpu_seconds": cpu_seconds() - root_cpu,
                "wall_seconds": root_clock.elapsed().as_secs_f64(),
                "sessions": inventory.sessions.len(), "uncertainties": inventory.uncertainties.len()}));
        }
    }
    let total_cpu = cpu_seconds() - cpu;
    let total_wall = clock.elapsed().as_secs_f64();
    let metrics = METRICS
        .lock()
        .unwrap()
        .iter()
        .map(|(name, (calls, cpu, wall))| {
            (
                (*name).to_owned(),
                serde_json::json!({"calls": calls, "cpu_seconds": cpu, "wall_seconds": wall}),
            )
        })
        .collect::<BTreeMap<_, _>>();
    println!(
        "{}",
        serde_json::json!({"pair": pair, "mode": mode,
        "tick_cpu_seconds": total_cpu, "tick_wall_seconds": total_wall,
        "capture_cpu_seconds": capture_cpu, "capture_wall_seconds": capture_wall,
        "roots": root_rows, "breakdown": metrics})
    );
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    assert!(args.len() >= 2, "usage: probe <sessions-dir> <root>...");
    for pair in 1..=3 {
        let order = if pair % 2 == 1 {
            ["before", "after"]
        } else {
            ["after", "before"]
        };
        for mode in order {
            run(mode, pair, Path::new(&args[0]), &args[1..]);
        }
    }
}
