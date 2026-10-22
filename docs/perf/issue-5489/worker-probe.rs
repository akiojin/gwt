//! Run the actual Monitor worker against real read-only sessions and isolated prefs.
#![allow(dead_code)]
pub use gwt::*;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};
#[path = "../../../target/issue-5489-probe/after.rs"]
mod after_inventory;
#[path = "../../../target/issue-5489-probe/before.rs"]
mod before_inventory;
#[path = "../../../target/issue-5489-probe/monitor-after.rs"]
mod monitor_after;
#[path = "../../../target/issue-5489-probe/monitor-before.rs"]
mod monitor_before;

// The copied modules define their own observation type. This adapter preserves
// the external API's type; empty PTY registrations never enter this stop path.
pub fn census_uncertainty<T: serde::Serialize>(value: &T, sessions: &Path, issue: u64) -> bool {
    let external = serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap();
    gwt::cli::execution_state::monitor_runtime_uncertainty_affects_issue(&external, sessions, issue)
}

#[repr(C)]
#[derive(Default)]
struct FileTime {
    low: u32,
    high: u32,
}
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentThread() -> *mut std::ffi::c_void;
    fn GetCurrentThreadId() -> u32;
    fn GetThreadTimes(
        thread: *mut std::ffi::c_void,
        created: *mut FileTime,
        exited: *mut FileTime,
        kernel: *mut FileTime,
        user: *mut FileTime,
    ) -> i32;
}
fn thread_cpu() -> f64 {
    let (mut created, mut exited, mut kernel, mut user) = (
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
        FileTime::default(),
    );
    // SAFETY: the pseudo-handle is this callback's worker thread and pointers are valid.
    assert_ne!(
        unsafe {
            GetThreadTimes(
                GetCurrentThread(),
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
fn prepare(fixture: &Path, roots: &[String]) {
    std::fs::create_dir(fixture).unwrap();
    let mut manifest = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let source = gwt::issue_monitor_prefs_path_for_repo_path(Path::new(root));
        let bytes = std::fs::read(&source).unwrap();
        std::fs::write(fixture.join(format!("{index}.json")), &bytes).unwrap();
        manifest.push(serde_json::json!({"root": root, "source_prefs": source,
            "bytes": bytes.len(), "sha256": format!("{:x}", Sha256::digest(&bytes))}));
    }
    std::fs::write(
        fixture.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
}
fn worker(mode: &str, fixture: &Path, sessions: &Path, roots: &[String]) {
    assert!(matches!(mode, "before" | "after"));
    let sandbox_home =
        std::env::temp_dir().join(format!("gwt-5489-worker-{mode}-{}", std::process::id()));
    std::fs::create_dir(&sandbox_home).unwrap();
    // This diagnostic child has not spawned threads yet. Changes affect only it.
    std::env::set_var("HOME", &sandbox_home);
    std::env::set_var("USERPROFILE", &sandbox_home);
    let mut projects = BTreeMap::new();
    let mut prefs = Vec::new();
    for (index, root) in roots.iter().enumerate() {
        let destination = gwt::issue_monitor_prefs_path_for_repo_path(Path::new(root));
        assert!(
            destination.starts_with(&sandbox_home),
            "prefs escaped isolation"
        );
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        let bytes = std::fs::read(fixture.join(format!("{index}.json"))).unwrap();
        std::fs::write(&destination, &bytes).unwrap();
        prefs.push(serde_json::json!({"path": destination,
            "sha256": format!("{:x}", Sha256::digest(&bytes))}));
        projects.insert(PathBuf::from(root), sessions.to_path_buf());
    }
    println!(
        "{}",
        serde_json::json!({"kind": "context", "mode": mode,
        "pid": std::process::id(), "isolated_home": sandbox_home, "fixture": fixture,
        "sessions_dir": sessions, "roots": roots, "initial_prefs": prefs,
        "pty_registrations": 0, "stop_or_park_groups": 0})
    );
    let (done, received) = std::sync::mpsc::channel();
    let state = std::sync::Mutex::new((None, 0));
    let mode_name = mode.to_owned();
    let callback = move || {
        let cpu = thread_cpu();
        let now = Instant::now();
        let mut state = state.lock().unwrap();
        if let Some((old_cpu, old_time)) = state.0 {
            let elapsed: Duration = now - old_time;
            state.1 += 1;
            println!(
                "{}",
                serde_json::json!({"kind": "tick", "mode": mode_name,
                    "tick": state.1, "thread_id": unsafe { GetCurrentThreadId() },
                "thread_name": std::thread::current().name(),
                "thread_cpu_seconds": cpu - old_cpu, "elapsed_seconds": elapsed.as_secs_f64(),
                "thread_cores": (cpu - old_cpu) / elapsed.as_secs_f64()})
            );
            if state.1 == 3 {
                done.send(()).unwrap();
            }
        }
        state.0 = Some((cpu, now));
    };
    let projects = Arc::new(RwLock::new(projects));
    if mode == "before" {
        monitor_before::spawn(projects, move || {
            callback();
            Some(Vec::new())
        });
    } else {
        monitor_after::spawn(projects, move || {
            callback();
            Some(Vec::new())
        });
    }
    received
        .recv_timeout(Duration::from_secs(90))
        .expect("worker did not produce three ticks");
}
fn main() {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("prepare") {
        assert_eq!(args.len(), 7);
        prepare(Path::new(&args[1]), &args[2..]);
    } else {
        assert_eq!(args.len(), 8);
        worker(
            &args[0],
            Path::new(&args[1]),
            Path::new(&args[2]),
            &args[3..],
        );
    }
}
