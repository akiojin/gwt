//! Issue #1764 — a tray/GUI front-door startup failure must stay diagnosable
//! after the process is gone.
//!
//! `crates/gwt/src/main.rs` is built with `windows_subsystem = "windows"` and
//! only attaches a parent console for non-GUI routes, so every `eprintln!` on
//! the tray startup path is written to a detached handle and discarded. The
//! canonical project log (`~/.gwt/projects/<hash>/logs/gwt.log.<date>`) is
//! therefore the only surface a user can inspect once the process has exited,
//! and every fatal front-door failure has to land there.
//!
//! These tests spawn the real `gwt` binary against an isolated `HOME` so they
//! exercise the production startup ordering rather than a helper in isolation.
//! They are platform-independent on purpose: the failure paths are shared, and
//! only the *visibility* of their diagnostics is Windows-specific.

use gwt::gui_single_instance::{acquire_instance_lock, LockKind};
use gwt_core::process::{hidden_command, scrub_git_env};
use std::{
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};

/// Generous upper bound: the front door hydrates the host `PATH` (which can
/// spawn a login shell) before it reaches any of the failure paths under test.
const EXIT_TIMEOUT: Duration = Duration::from_secs(90);

struct FrontDoorExit {
    code: Option<i32>,
    stderr: String,
}

/// Spawn the real front door with an isolated `HOME` and wait for it to exit.
///
/// The child is killed if it outlives [`EXIT_TIMEOUT`]; without that guard a
/// regression that lets startup succeed would hang the suite inside the `tao`
/// event loop instead of failing.
fn run_front_door(home: &Path, workspace: &Path, extra_args: &[&str]) -> FrontDoorExit {
    let stderr_path = workspace.join("front-door-stderr.log");
    let stderr = std::fs::File::create(&stderr_path).expect("create stderr capture");

    let mut command = hidden_command(env!("CARGO_BIN_EXE_gwt"));
    scrub_git_env(&mut command);
    command
        .args(["--no-tray", "--no-open"])
        .args(extra_args)
        .current_dir(workspace)
        .env("HOME", home)
        .env("USERPROFILE", home)
        .env("XDG_CONFIG_HOME", home.join("xdg-config"))
        .env("XDG_CACHE_HOME", home.join("xdg-cache"))
        .env("XDG_DATA_HOME", home.join("xdg-data"))
        .env("XDG_STATE_HOME", home.join("xdg-state"))
        .env("CI", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("RUST_LOG")
        .env_remove("GWT_FORCE_NEW_INSTANCE")
        .env_remove("GWT_SESSION_ID")
        .env_remove("GWT_PROJECT_ROOT")
        .env_remove("GWT_PROJECT_ROOT_HASH")
        .env_remove("GWT_WORKTREE_HASH")
        .env_remove("GWT_WORKSPACE_ID")
        .env_remove("GWT_RUNTIME_DIR")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));

    let mut child = command.spawn().expect("spawn isolated gwt front door");
    let status = wait_with_deadline(&mut child);
    FrontDoorExit {
        code: status,
        stderr: std::fs::read_to_string(&stderr_path).unwrap_or_default(),
    }
}

fn wait_with_deadline(child: &mut Child) -> Option<i32> {
    let deadline = Instant::now() + EXIT_TIMEOUT;
    loop {
        match child.try_wait().expect("inspect front door child") {
            Some(status) => return status.code(),
            None if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "gwt front door did not exit within {EXIT_TIMEOUT:?}; \
                     the startup failure path under test was not reached"
                );
            }
            None => std::thread::sleep(Duration::from_millis(25)),
        }
    }
}

/// Concatenate every rolling log file the child wrote under the isolated home.
///
/// The directory is discovered rather than recomputed from the project hash so
/// the test asserts the user-visible outcome ("a log exists and explains the
/// failure") instead of restating the path-derivation implementation.
fn read_project_logs(home: &Path) -> String {
    let projects = home.join(".gwt").join("projects");
    let mut combined = String::new();
    for log_file in collect_log_files(&projects) {
        if let Ok(contents) = std::fs::read_to_string(&log_file) {
            combined.push_str(&contents);
        }
    }
    combined
}

fn collect_log_files(projects_dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(projects) = std::fs::read_dir(projects_dir) else {
        return found;
    };
    for project in projects.flatten() {
        let Ok(logs) = std::fs::read_dir(project.path().join("logs")) else {
            continue;
        };
        for entry in logs.flatten() {
            let path = entry.path();
            if path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(gwt_core::logging::LOG_FILE_BASENAME))
            {
                found.push(path);
            }
        }
    }
    found
}

fn isolated_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().expect("fixture tempdir");
    let home = temp.path().join("home");
    let workspace = temp.path().join("workspace");
    std::fs::create_dir_all(home.join(".gwt")).expect("create isolated gwt home");
    std::fs::create_dir_all(&workspace).expect("create isolated workspace");
    (temp, home, workspace)
}

/// The single-instance collision is the exact failure Issue #1764's
/// `GWT_FORCE_NEW_INSTANCE` escape hatch was added to recover from: on Windows
/// a crashed predecessor (or security software pinning the handle) can leave
/// the OS lock held, and the next launch dies at `exit(2)`. Both the failure
/// and the recovery hint have to reach the log, otherwise the user sees gwt
/// vanish with no explanation anywhere.
#[test]
fn single_instance_lock_collision_is_recorded_in_the_project_log() {
    let (_temp, home, workspace) = isolated_fixture();

    let _held = acquire_instance_lock(&home.join(".gwt"), &workspace, LockKind::Gui)
        .expect("test harness must own the single-instance lock first");

    let exit = run_front_door(&home, &workspace, &[]);
    assert_eq!(
        exit.code,
        Some(2),
        "a held single-instance lock must fail the front door with exit 2 (stderr: {})",
        exit.stderr
    );

    let logged = read_project_logs(&home);
    assert!(
        logged.contains("already running"),
        "the single-instance failure must be recoverable from the project log, \
         but the log did not mention it.\nlog:\n{logged}\nstderr:\n{}",
        exit.stderr
    );
    assert!(
        logged.contains("GWT_FORCE_NEW_INSTANCE"),
        "the log must carry the recovery hint so the escape hatch is discoverable \
         on a console-less Windows launch.\nlog:\n{logged}\nstderr:\n{}",
        exit.stderr
    );
}

/// Argv rejection happens even earlier than the lock, so it pins the ordering
/// contract: logging is initialised before anything that can terminate the
/// front door.
#[test]
fn invalid_front_door_flag_failure_is_recorded_in_the_project_log() {
    let (_temp, home, workspace) = isolated_fixture();

    let exit = run_front_door(&home, &workspace, &["--no-such-flag"]);
    assert_eq!(
        exit.code,
        Some(2),
        "an unknown front-door flag must exit 2 (stderr: {})",
        exit.stderr
    );

    let logged = read_project_logs(&home);
    assert!(
        logged.contains("--no-such-flag"),
        "the rejected flag must be recoverable from the project log.\nlog:\n{logged}\nstderr:\n{}",
        exit.stderr
    );
}
