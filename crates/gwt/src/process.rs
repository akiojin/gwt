//! Cross-platform process liveness probe shared by the daemon
//! bootstrap callers.
//!
//! This module centralises the `kill(pid, 0)` probe that several
//! daemon-related modules (`cli::daemon::mod`, `daemon_publisher`,
//! `main`) used to duplicate. Three identical 10-line helpers had
//! drifted slightly (`is_process_alive_pid`, `is_alive`,
//! `is_subscriber_pid_alive`); consolidating into one definition
//! removes that drift surface and makes the platform-conditional
//! behaviour explicit in a single place.
//!
//! Every daemon-bootstrap caller now shares this one predicate. The GUI front
//! door used to run a narrower `|pid| pid == std::process::id()` variant that
//! classified a live daemon as dead; Issue #2338 resolved that by removing the
//! front door's endpoint-slot handling entirely rather than by giving it a
//! second liveness definition.

/// Return `true` when `pid` refers to a live process visible to the
/// current user on a Unix host.
///
/// On non-Unix targets (Windows today), the daemon's `serve_blocking`
/// is a stub, so reporting any persisted endpoint as "alive" would
/// surface permanent stale entries in `gwtd daemon status`. Returning
/// `false` lets `resolve_bootstrap_action` treat such endpoints as
/// dead and clean them up on the next bootstrap call.
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, 0) returns 0 if the process exists, -1
        // with ESRCH if it does not. We never deliver a real signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        // EPERM means the process exists but we lack permission to
        // signal it — still alive from the bootstrap caller's POV.
        matches!(err.raw_os_error(), Some(libc::EPERM))
    }
    #[cfg(not(unix))]
    {
        // Windows named-pipe support for the daemon is a follow-up.
        // When that lands, this branch should switch to a real
        // liveness probe (e.g. `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION,
        // ...)`).
        false
    }
}

/// Return whether a GUI materializer process is alive on every supported host.
/// This is intentionally separate from [`is_process_alive`]: the latter keeps
/// Windows daemon-bootstrap compatibility semantics while launch-delivery
/// leases need a real cross-platform owner probe.
pub fn is_host_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        is_process_alive(pid)
    }
    #[cfg(not(unix))]
    {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

        let mut system = System::new();
        let pid = sysinfo::Pid::from_u32(pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        system.process(pid).is_some()
    }
}

/// Return the OS-reported start time for one host process.
///
/// A PID by itself is not a durable process identity because operating
/// systems recycle it. Cross-process launch fences persist this value beside
/// the PID and compare both before treating a previous Host as still live.
pub fn host_process_start_time(pid: u32) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system
        .process(pid)
        .map(sysinfo::Process::start_time)
        .filter(|started_at| *started_at > 0)
}

/// Return whether a Unix process group still contains any process.
///
/// PTY children are session/process-group leaders. The direct leader can exit
/// while a foreground descendant remains the execution writer, so PID
/// liveness alone is not sufficient recovery evidence. Windows process trees
/// are owned by the kill-on-close Job Object and use direct child identity.
pub fn is_process_group_alive(process_group_id: u32) -> bool {
    if process_group_id == 0 || process_group_id > i32::MAX as u32 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 performs only an existence/permission probe. A
        // negative pid addresses the whole process group.
        let rc = unsafe { libc::kill(-(process_group_id as libc::pid_t), 0) };
        if rc == 0 {
            return true;
        }
        matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        )
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Check the exact PTY child or any surviving member of its process group.
pub fn exact_pty_process_tree_is_alive(child_pid: u32, child_started_at: u64) -> bool {
    host_process_start_time(child_pid) == Some(child_started_at)
        || is_process_group_alive(child_pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_zero_is_never_alive() {
        assert!(!is_process_alive(0));
        assert!(!is_host_process_alive(0));
    }

    #[test]
    fn current_host_process_is_alive_on_every_supported_platform() {
        assert!(is_host_process_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn current_unix_process_group_is_detected() {
        // SAFETY: getpgrp has no preconditions or side effects.
        let process_group = unsafe { libc::getpgrp() };
        assert!(process_group > 0);
        assert!(is_process_group_alive(process_group as u32));
        assert!(!is_process_group_alive(0));
    }

    #[test]
    fn host_process_start_time_distinguishes_the_current_process_from_missing_pid() {
        assert!(host_process_start_time(std::process::id()).is_some_and(|value| value > 0));
        assert_eq!(host_process_start_time(0), None);
        assert_eq!(host_process_start_time(i32::MAX as u32), None);
    }

    #[cfg(unix)]
    #[test]
    fn current_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn far_unused_pid_is_not_alive() {
        // Use `i32::MAX as u32` so the value stays positive after the
        // `pid as libc::pid_t` cast inside `is_process_alive`. Going
        // higher (e.g. `u32::MAX - 1`) wraps to a negative `pid_t` and
        // `kill(-N, 0)` probes process *group* `N` instead of a far
        // PID, which is a different semantic and can flake on
        // runners where group 2 exists.
        //
        // `i32::MAX` (~2.1 billion) is far past any realistic OS
        // pid_t allocation window today; if this ever flakes on a CI
        // runner we'll have learned that pid recycling has reached
        // extreme territory.
        assert!(!is_process_alive(i32::MAX as u32));
    }
}
