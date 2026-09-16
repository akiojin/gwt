//! Launching a verification workload from the daemon (Issue #4409).
//!
//! `gwtd` lives inside the agent's PTY process tree and therefore hands every
//! command it spawns the agent launch policy's degraded priority. A
//! non-privileged process cannot lower its own nice value, so the only way to
//! give verification baseline priority is to have a process that already has
//! it do the spawning. The daemon under the GUI is that process.
//!
//! Escaping a process tree creates the orphan problem #3845 was filed for, so
//! reclamation is part of the spawn rather than a follow-up: the child leads
//! its own session, and [`VerificationChild`] kills that whole group when it
//! is dropped — which happens when the child exits, when the requesting
//! connection closes, and when the daemon shuts down. The group is the child's
//! own, so the daemon is never inside its own blast radius (AC-7).

use gwt_core::daemon::{VerificationSpawnAccepted, VerificationSpawnRequest};
// Every reader of this lives on a Unix-only path — `spawn` and the reason it
// records — so on Windows the import itself is what `-D warnings` catches.
#[cfg(unix)]
use gwt_core::verification_priority::BASELINE_NICE;

/// A running verification child plus the reclamation obligation it carries.
///
/// Dropping the value kills the child's process group. Every exit path from
/// the daemon's connection handler therefore reclaims, including the panicking
/// and cancelled ones that an explicit cleanup call would miss.
pub struct VerificationChild {
    child: std::process::Child,
    process_group: u32,
    accepted: VerificationSpawnAccepted,
    reaped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl VerificationChild {
    /// What to report back to the requesting client.
    pub fn accepted(&self) -> &VerificationSpawnAccepted {
        &self.accepted
    }

    /// A reclamation obligation that can be held somewhere else.
    ///
    /// The daemon parks this with the *connection* while the child itself is
    /// waited on from a blocking task. That split is what binds the child's
    /// lifetime to the requester: when the connection ends — the caller
    /// disconnected, `gwtd` died, the pane closed — the handle drops, the
    /// group is killed, and the blocking wait returns on its own.
    pub fn reclaim_handle(&self) -> VerificationReclaim {
        VerificationReclaim {
            process_group: self.process_group,
            reaped: std::sync::Arc::clone(&self.reaped),
        }
    }

    /// Block until the child exits, then reclaim anything it left running.
    ///
    /// Reclamation happens *before* the child is reaped. A process group id
    /// stays allocated while any member exists, and the just-exited child is
    /// still a zombie member until `wait` collects it, so killing the group at
    /// this point cannot land on a recycled group id.
    pub fn wait(mut self) -> (i32, bool) {
        let exit_code = match self.child.wait() {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        };
        // Descendants outlive the runner often enough to be the normal case:
        // a `cargo test` that exits while a test binary is still winding down
        // is exactly the shape that hung the full suite in #3845.
        let reclaimed_survivors = reclaim_group(self.process_group);
        self.reaped.store(true, std::sync::atomic::Ordering::SeqCst);
        (exit_code, reclaimed_survivors)
    }
}

impl Drop for VerificationChild {
    fn drop(&mut self) {
        // `wait` consumes `self`, so reaching here means the child was
        // abandoned: the daemon is shutting down, or a handler unwound. Kill
        // first and reap after, for the same group-id-reuse reason documented
        // on `wait`.
        if !self.reaped.load(std::sync::atomic::Ordering::SeqCst) {
            reclaim_group(self.process_group);
            let _ = self.child.wait();
        }
    }
}

/// The right to reclaim a verification child's process group, held apart from
/// the child itself.
pub struct VerificationReclaim {
    process_group: u32,
    reaped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Drop for VerificationReclaim {
    fn drop(&mut self) {
        // Skip a group whose leader has already been reaped: its id is free
        // to be recycled at that point, and killing a recycled group would
        // take down an unrelated process.
        if !self.reaped.load(std::sync::atomic::Ordering::SeqCst) {
            reclaim_group(self.process_group);
        }
    }
}

/// Kill a whole process group. Returns whether anything was still alive.
#[cfg(unix)]
fn reclaim_group(process_group: u32) -> bool {
    // SAFETY: `killpg` has no memory-safety preconditions. An `ESRCH` result
    // means the group is already empty, which is the ordinary outcome for a
    // command that cleaned up after itself.
    unsafe { libc::killpg(process_group as libc::pid_t, libc::SIGKILL) == 0 }
}

#[cfg(not(unix))]
fn reclaim_group(_process_group: u32) -> bool {
    false
}

/// Launch one verification command in a dedicated process group at baseline
/// priority.
#[cfg(unix)]
pub fn spawn(request: &VerificationSpawnRequest) -> Result<VerificationChild, String> {
    use std::os::unix::process::CommandExt;

    let stdout = open_transcript(&request.stdout_path)?;
    let stderr = open_transcript(&request.stderr_path)?;

    // The caller sends its complete environment because the daemon's own is
    // not the caller's, and a test runner that sees a different environment
    // produces verdicts nobody can reproduce — hence `inherit_env(false)`.
    let plan = request.env.iter().fold(
        gwt_core::process::ProcessPlanRequest::new(&request.program)
            .args(&request.args)
            .current_dir(&request.cwd)
            .inherit_env(false),
        |plan, (key, value)| plan.env(key, value),
    );
    let mut command = gwt_core::process::resolved_command(plan)
        .map_err(|err| format!("failed to resolve '{}': {err}", request.program))?;
    command
        .stdin(std::process::Stdio::null())
        .stdout(stdout)
        .stderr(stderr);

    // SAFETY: only async-signal-safe calls run between fork and exec.
    unsafe {
        command.pre_exec(|| {
            // A new session makes the child a process group leader, which is
            // both the escape from the caller's group and the handle
            // reclamation needs.
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            // Best effort by design: a daemon that is itself degraded cannot
            // lower its child either. That is an environment fact the run
            // records rather than a reason to refuse it (AC-6), so the
            // failure is read back below instead of aborting the spawn.
            libc::setpriority(libc::PRIO_PROCESS as _, 0, BASELINE_NICE);
            Ok(())
        });
    }

    let child = command
        .spawn()
        .map_err(|err| format!("failed to spawn '{}': {err}", request.program))?;
    let pid = child.id();
    let nice = observed_nice(pid);

    Ok(VerificationChild {
        child,
        // `setsid` makes the process group id equal the child's pid.
        process_group: pid,
        accepted: VerificationSpawnAccepted {
            pid,
            process_group: pid,
            nice,
            nice_reason: priority_reason(nice, degraded_qos_class()),
        },
        reaped: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    })
}

/// The scheduling class a macOS child inherits from the daemon, when that
/// class is worse than the work deserves.
///
/// A child inherits the daemon's task QoS, and a `UTILITY` task has its timers
/// coalesced aggressively: measured on this host, fifty 20ms sleeps took 7.2s
/// from a `UTILITY` daemon against 1.3s from a `USER_INTERACTIVE` one. That is
/// the same kind of inherited penalty as `nice` and a larger one, and it
/// cannot be undone from inside the child — raising the spawning thread's
/// class does not reach the child's task, and raising the child's own thread
/// class leaves the task's timer coalescing in place. Both were measured.
///
/// So this is reported rather than corrected, for the same reason an
/// unreachable nice value is (AC-6): it is a property of how the daemon was
/// launched, which the caller cannot fix but does need to see. A daemon owned
/// by the gwt GUI runs at `USER_INTERACTIVE` and needs none of this; one
/// started by `launchctl submit` runs at `UTILITY` and silently makes every
/// verification slower than it was before it escaped.
#[cfg(target_os = "macos")]
fn degraded_qos_class() -> Option<&'static str> {
    // SAFETY: `qos_class_self` has no memory-safety preconditions.
    let class = unsafe { qos_class_self() };
    match class {
        QOS_CLASS_BACKGROUND => Some("BACKGROUND"),
        QOS_CLASS_UTILITY => Some("UTILITY"),
        _ => None,
    }
}

// Only `spawn` reads this, and `spawn` exists as a real implementation on Unix
// alone — Windows gets the stub below. Without the `unix` half of the gate the
// function is unreachable there and `-D warnings` fails the Windows Lint job
// while every macOS and Linux check stays green.
#[cfg(all(unix, not(target_os = "macos")))]
fn degraded_qos_class() -> Option<&'static str> {
    None
}

#[cfg(target_os = "macos")]
extern "C" {
    fn qos_class_self() -> u32;
}

/// `QOS_CLASS_UTILITY` / `QOS_CLASS_BACKGROUND` from `<sys/qos.h>`.
#[cfg(target_os = "macos")]
const QOS_CLASS_UTILITY: u32 = 0x11;
#[cfg(target_os = "macos")]
const QOS_CLASS_BACKGROUND: u32 = 0x09;

#[cfg(not(unix))]
pub fn spawn(_request: &VerificationSpawnRequest) -> Result<VerificationChild, String> {
    Err(
        "daemon-hosted verification spawn is a Unix path; on Windows the launcher \
         escapes the agent's job object directly (Issue #4405)"
            .to_string(),
    )
}

/// Explain a child that did not inherit the scheduling treatment the work
/// deserves (AC-6). `None` means it did.
///
/// Unix-only for the same reason as [`degraded_qos_class`]: its only caller is
/// the Unix `spawn`.
#[cfg(unix)]
fn priority_reason(nice: Option<i32>, degraded_qos: Option<&str>) -> Option<String> {
    let mut reasons = Vec::new();
    match nice {
        Some(nice) if nice > BASELINE_NICE => reasons.push(format!(
            "the daemon could not lower the child to nice {BASELINE_NICE}: it runs at nice \
             {nice} itself and setpriority(2) refuses to raise a process's priority without \
             privilege"
        )),
        Some(_) => {}
        None => reasons.push("the child's nice value could not be read back".to_string()),
    }
    if let Some(class) = degraded_qos {
        reasons.push(format!(
            "the daemon runs in the {class} QoS class and the child inherits it, which coalesces \
             its timers — measured at 5x longer short sleeps than a USER_INTERACTIVE parent. A \
             daemon owned by the gwt GUI does not have this; one started by `launchctl submit` \
             does. Restart the daemon from the GUI to clear it"
        ));
    }
    if reasons.is_empty() {
        return None;
    }
    Some(format!(
        "{}. The workload runs anyway at what it inherited; expect it to be slower than a \
         command started from a terminal",
        reasons.join("; and ")
    ))
}

#[cfg(unix)]
fn observed_nice(pid: u32) -> Option<i32> {
    // SAFETY: `getpriority` has no memory-safety preconditions.
    Some(unsafe { libc::getpriority(libc::PRIO_PROCESS as _, pid as libc::id_t) })
}

#[cfg(unix)]
fn open_transcript(path: &std::path::Path) -> Result<std::fs::File, String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("failed to create {}: {err}", parent.display()))?;
    }
    std::fs::File::create(path).map_err(|err| format!("failed to create {}: {err}", path.display()))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn request(dir: &std::path::Path, program: &str, args: &[&str]) -> VerificationSpawnRequest {
        VerificationSpawnRequest {
            program: program.to_string(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            cwd: dir.to_path_buf(),
            env: std::env::vars().collect(),
            stdout_path: dir.join("stdout.log"),
            stderr_path: dir.join("stderr.log"),
        }
    }

    fn alive(pid: u32) -> bool {
        // Signal 0 probes for existence without delivering anything.
        unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
    }

    fn wait_until_gone(pid: u32) -> bool {
        for _ in 0..200 {
            if !alive(pid) {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        !alive(pid)
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "gwt-4409-{name}-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    /// AC-1: the child must not sit in the launcher's process group, because
    /// that group is the agent's and carries the priority being escaped.
    #[test]
    fn the_child_leads_its_own_process_group() {
        let dir = temp_dir("group");
        let child = spawn(&request(&dir, "/bin/sh", &["-c", "sleep 5"])).expect("spawn");
        let accepted = child.accepted().clone();
        let own_group = unsafe { libc::getpgrp() } as u32;

        assert_eq!(
            accepted.process_group, accepted.pid,
            "setsid makes the child its own group leader"
        );
        assert_ne!(
            accepted.process_group, own_group,
            "a child sharing the launcher's group never escaped it"
        );
        drop(child);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC-2 / AC-7: abandoning the child — which is what a closed connection
    /// and a dead `gwtd` both look like from here — reclaims the whole group,
    /// grandchildren included. #3845 is the regression this pins.
    #[test]
    fn dropping_the_child_reclaims_grandchildren_too() {
        let dir = temp_dir("orphans");
        // The shell exits immediately while `sleep` keeps running, so the
        // survivor is a grandchild that only group-wide reclamation reaches.
        let child = spawn(&request(
            &dir,
            "/bin/sh",
            &["-c", "sleep 120 & echo $! > grandchild.pid; sleep 120"],
        ))
        .expect("spawn");
        let child_pid = child.accepted().pid;

        let grandchild = dir.join("grandchild.pid");
        let mut grandchild_pid = None;
        for _ in 0..200 {
            if let Ok(text) = std::fs::read_to_string(&grandchild) {
                if let Ok(pid) = text.trim().parse::<u32>() {
                    grandchild_pid = Some(pid);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }
        let grandchild_pid = grandchild_pid.expect("grandchild recorded its pid");
        assert!(alive(grandchild_pid), "grandchild should be running");

        drop(child);

        assert!(
            wait_until_gone(child_pid),
            "the child must not survive its owner"
        );
        assert!(
            wait_until_gone(grandchild_pid),
            "a grandchild left behind is exactly the #3845 hang; group reclamation must reach it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC-2, the path the daemon actually takes: the child is waited on from a
    /// blocking task while the *connection* holds the reclamation handle.
    /// Dropping that handle — a disconnected or dead caller — must stop the
    /// workload even though the child itself is owned elsewhere.
    #[test]
    fn dropping_the_connections_reclaim_handle_stops_the_child() {
        let dir = temp_dir("handle");
        let child = spawn(&request(&dir, "/bin/sh", &["-c", "sleep 120"])).expect("spawn");
        let child_pid = child.accepted().pid;
        let handle = child.reclaim_handle();
        let waiter = std::thread::spawn(move || child.wait());

        assert!(alive(child_pid), "child should be running");
        drop(handle);

        let (exit_code, _) = waiter.join().expect("waiter thread");
        assert_eq!(
            exit_code, -1,
            "a SIGKILLed child reports no exit code of its own"
        );
        assert!(wait_until_gone(child_pid));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The handle must not fire once the child has been reaped: at that point
    /// the group id is free and killing it could reach an unrelated process.
    #[test]
    fn a_reclaim_handle_outliving_a_finished_child_is_inert() {
        let dir = temp_dir("inert");
        let child = spawn(&request(&dir, "/bin/sh", &["-c", "exit 0"])).expect("spawn");
        let handle = child.reclaim_handle();
        let (exit_code, _) = child.wait();

        assert_eq!(exit_code, 0);
        assert!(
            handle.reaped.load(std::sync::atomic::Ordering::SeqCst),
            "a reaped child must disarm the handle before it drops"
        );
        drop(handle);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The daemon must not be caught in its own cleanup: reclamation targets
    /// the child's group, and the daemon is in a different one (AC-7).
    #[test]
    fn reclamation_never_targets_the_daemons_own_group() {
        let dir = temp_dir("self");
        let child = spawn(&request(&dir, "/bin/sh", &["-c", "sleep 5"])).expect("spawn");
        assert_ne!(
            child.accepted().process_group,
            unsafe { libc::getpgrp() } as u32,
            "killing this group must not be able to reach the daemon"
        );
        drop(child);
        assert!(alive(std::process::id()), "the daemon survived reclamation");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn waiting_reports_the_exit_code_and_captures_the_transcript() {
        let dir = temp_dir("exit");
        let child = spawn(&request(
            &dir,
            "/bin/sh",
            &["-c", "echo out; echo err 1>&2; exit 7"],
        ))
        .expect("spawn");
        let (exit_code, _) = child.wait();

        assert_eq!(exit_code, 7);
        assert_eq!(
            std::fs::read_to_string(dir.join("stdout.log")).expect("stdout"),
            "out\n"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("stderr.log")).expect("stderr"),
            "err\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// AC-6: a nice value that could not be lowered is recorded, not refused.
    /// The reason has to name the cause, because the reader's next question is
    /// whether they can do anything about it.
    #[test]
    fn an_unreachable_baseline_is_recorded_with_its_cause() {
        assert_eq!(priority_reason(Some(BASELINE_NICE), None), None);
        let reason = priority_reason(Some(10), None).expect("a degraded child explains itself");
        assert!(reason.contains("nice 10"), "{reason}");
        assert!(reason.contains("setpriority"), "{reason}");
        assert!(
            reason.contains("runs anyway at what it inherited"),
            "the record must say the run continued: {reason}"
        );
    }

    /// A degraded QoS class is the larger inherited penalty on macOS and
    /// cannot be undone from inside the child, so it has to be visible in the
    /// record rather than silently making every run slower.
    #[test]
    fn an_inherited_utility_qos_class_is_recorded_with_what_to_do_about_it() {
        let reason = priority_reason(Some(BASELINE_NICE), Some("UTILITY"))
            .expect("a child at baseline nice but degraded QoS still explains itself");
        assert!(reason.contains("UTILITY"), "{reason}");
        assert!(reason.contains("coalesces"), "{reason}");
        assert!(
            reason.contains("Restart the daemon from the GUI"),
            "the record must name the next operation: {reason}"
        );
        let both = priority_reason(Some(10), Some("BACKGROUND")).expect("both penalties");
        assert!(
            both.contains("nice 10") && both.contains("BACKGROUND"),
            "{both}"
        );
    }

    /// The child must see the caller's environment, not the daemon's.
    #[test]
    fn the_child_environment_comes_from_the_request() {
        let dir = temp_dir("env");
        let mut req = request(
            &dir,
            "/bin/sh",
            &["-c", "echo \"${GWT_4409_MARKER:-unset}\""],
        );
        req.env = vec![
            ("PATH".to_string(), "/usr/bin:/bin".to_string()),
            ("GWT_4409_MARKER".to_string(), "from-caller".to_string()),
        ];
        let child = spawn(&req).expect("spawn");
        let (exit_code, _) = child.wait();

        assert_eq!(exit_code, 0);
        assert_eq!(
            std::fs::read_to_string(dir.join("stdout.log")).expect("stdout"),
            "from-caller\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
