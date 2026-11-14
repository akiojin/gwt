//! Lifetime of a transient `--no-tray` front door (Issue #5219).
use std::time::{Duration, Instant};
use sysinfo::{Pid, ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

pub const POLL_INTERVAL: Duration = Duration::from_millis(500);
const RECONNECT_GRACE: Duration = Duration::from_secs(5);

/// Capture before bootstrap so an orphaned launch cannot adopt its new parent.
pub struct ParentProcess(Option<(Pid, u64)>);

impl ParentProcess {
    pub fn capture() -> Self {
        let mut system = System::new();
        let Ok(current) = sysinfo::get_current_pid() else {
            return Self(None);
        };
        refresh(&mut system, current);
        let Some(parent) = system.process(current).and_then(|process| process.parent()) else {
            return Self(None);
        };
        if parent.as_u32() <= 1 {
            return Self(None);
        }
        refresh(&mut system, parent);
        Self(
            system
                .process(parent)
                .map(|process| (parent, process.start_time())),
        )
    }

    pub fn is_alive(&self) -> bool {
        let Some((pid, started)) = self.0 else {
            return false;
        };
        #[cfg(unix)]
        if std::os::unix::process::parent_id() != pid.as_u32() {
            return false;
        }
        // A fresh snapshot also detects PID reuse, rather than retaining the
        // old process metadata in a long-lived System cache.
        let mut system = System::new();
        refresh(&mut system, pid);
        system.process(pid).is_some_and(|process| {
            process.start_time() == started
                && !matches!(
                    process.status(),
                    ProcessStatus::Dead | ProcessStatus::Zombie
                )
        })
    }
}

fn refresh(system: &mut System, pid: Pid) {
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
        assert!(!ParentProcess(None).is_alive());
        assert!(ParentProcess::capture().is_alive());
    }
}
