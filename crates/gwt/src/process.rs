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
//! Note: `prepare_daemon_front_door_for_path`
//! (`crates/gwt/src/cli/hook/mod.rs`) deliberately uses a *narrower*
//! predicate (`|pid| pid == std::process::id()`) and is kept inline
//! there. That difference is the subject of Issue #2338 — fixing it
//! requires SPEC-2077 owner alignment on endpoint-slot semantics, so
//! this module intentionally does not absorb that callsite.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_zero_is_never_alive() {
        assert!(!is_process_alive(0));
    }

    #[cfg(unix)]
    #[test]
    fn current_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn far_unused_pid_is_not_alive() {
        // u32::MAX - 1 is well past any realistic OS pid_t allocation
        // window; if this ever fails on a CI runner we'll know that
        // pid recycling has reached extreme territory.
        assert!(!is_process_alive(u32::MAX - 1));
    }
}
