//! Lifetime of a transient `--no-tray` front door (Issue #5219).
use std::time::{Duration, Instant};
#[cfg(not(windows))]
use sysinfo::ProcessStatus;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
const RECONNECT_GRACE: Duration = Duration::from_secs(5);

#[cfg(all(test, windows))]
thread_local! {
    static REFRESH_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Capture before bootstrap so an orphaned launch cannot adopt its new parent.
pub struct ParentProcess(Result<Option<(Pid, u64)>, sysinfo::Error>);

impl ParentProcess {
    pub fn capture() -> Self {
        Self(Self::capture_identity())
    }

    fn capture_identity() -> Result<Option<(Pid, u64)>, sysinfo::Error> {
        let mut system = System::new()?;
        let Ok(current) = sysinfo::get_current_pid() else {
            return Ok(None);
        };
        refresh(&mut system, current);
        let Some(parent) = system.process(current).and_then(|process| process.parent()) else {
            return Ok(None);
        };
        if parent.as_u32() <= 1 {
            return Ok(None);
        }
        refresh(&mut system, parent);
        Ok(system
            .process(parent)
            .map(|process| {
                (
                    parent,
                    crate::process::snapshot_process_start_identity(parent.as_u32(), process),
                )
            })
            .filter(|(_, started)| *started > 0))
    }

    pub fn is_alive(&self) -> bool {
        // Unavailable observation must not end the server as an orphan.
        let (pid, started) = match self.0 {
            Ok(Some(identity)) => identity,
            Ok(None) => return false,
            Err(_) => return true,
        };
        #[cfg(unix)]
        if std::os::unix::process::parent_id() != pid.as_u32() {
            return false;
        }
        #[cfg(windows)]
        {
            // Native probes read fresh handles without a machine-wide snapshot.
            // Only definite death or a different incarnation ends this owner.
            crate::process::host_process_liveness(pid.as_u32()) != Some(false)
                && crate::process::host_process_start_time(pid.as_u32())
                    .is_none_or(|current| current == started)
        }
        #[cfg(not(windows))]
        {
            // A fresh snapshot also detects PID reuse, rather than retaining the
            // old process metadata in a long-lived System cache.
            let Ok(mut system) = System::new() else {
                return true;
            };
            refresh(&mut system, pid);
            system.process(pid).is_some_and(|process| {
                // Linux epoch start times move with WSL btime; this opaque
                // token stays fixed for the same process incarnation.
                crate::process::snapshot_process_start_identity(pid.as_u32(), process) == started
                    && !matches!(
                        process.status(),
                        ProcessStatus::Dead | ProcessStatus::Zombie
                    )
            })
        }
    }
}

fn refresh(system: &mut System, pid: Pid) {
    #[cfg(all(test, windows))]
    REFRESH_COUNT.with(|count| count.set(count.get() + 1));
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing().without_tasks(),
    );
}

#[derive(Default)]
pub struct BrowserLifetime {
    disconnected_at: Option<Instant>,
    generation: u64,
}

impl BrowserLifetime {
    /// Never-connected servers follow their parent. A reload may reconnect
    /// before the grace expires; only the last browser connection counts.
    pub fn ended(&mut self, generation: u64, connected: bool, now: Instant) -> bool {
        if generation != self.generation {
            self.generation = generation;
            self.disconnected_at = None;
        }
        if connected || generation == 0 {
            self.disconnected_at = None;
            return false;
        }
        now.duration_since(*self.disconnected_at.get_or_insert(now)) >= RECONNECT_GRACE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_lifetime_waits_for_first_session_and_allows_reconnect() {
        let now = Instant::now();
        let mut lifetime = BrowserLifetime::default();
        assert!(!lifetime.ended(0, false, now));
        assert!(!lifetime.ended(0, false, now + RECONNECT_GRACE));
        assert!(!lifetime.ended(1, true, now));
        assert!(!lifetime.ended(1, false, now));
        assert!(!lifetime.ended(2, false, now + RECONNECT_GRACE));
        assert!(lifetime.ended(2, false, now + RECONNECT_GRACE * 2));
    }

    #[test]
    fn missing_parent_is_terminal_and_live_parent_is_detected() {
        assert!(!ParentProcess(Ok(None)).is_alive());
        assert!(ParentProcess(Err(sysinfo::Error::Unsupported)).is_alive());
        let parent = ParentProcess::capture();
        #[cfg(target_os = "linux")]
        {
            let (pid, started) = parent.0.as_ref().unwrap().unwrap();
            let native = crate::process::host_process_start_time(pid.as_u32()).unwrap();
            assert_eq!(
                started, native,
                "parent identity must use the shared opaque token"
            );
            assert!(!ParentProcess(Ok(Some((pid, native + 1)))).is_alive());
        }
        #[cfg(windows)]
        let refreshes_before_poll = REFRESH_COUNT.get();
        assert!(parent.is_alive());
        #[cfg(windows)]
        {
            let pid = Pid::from_u32(std::process::id());
            let started = crate::process::host_process_start_time(pid.as_u32()).unwrap();
            assert!(ParentProcess(Ok(Some((pid, started)))).is_alive());
            assert!(!ParentProcess(Ok(Some((pid, started - 1)))).is_alive());
            assert!(!ParentProcess(Ok(Some((Pid::from_u32(u32::MAX), started)))).is_alive());
            assert_eq!(
                REFRESH_COUNT.get(),
                refreshes_before_poll,
                "Windows parent polling must avoid full process snapshots"
            );
        }
    }
}
