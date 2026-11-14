//! Platform-specific process group management for PTY children.
//!
//! - Windows: Wraps the child in a Job Object with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so that closing the job handle
//!   terminates the child and every descendant it spawned.
//! - Unix: `portable_pty` already places the child in its own session
//!   (setsid), so the child's pid is also its process group id. On drop we
//!   send `SIGKILL` to the group via `killpg` without waiting (Issue #3705).

#[cfg(windows)]
mod imp {
    use super::super::{ProcessPriority, ProcessResourcePolicy};

    #[derive(Default)]
    pub struct ProcessGroup {
        job: Option<gwt_core::process_tree::WindowsJobObject>,
    }

    impl ProcessGroup {
        pub fn attach(pid: u32) -> Result<Self, String> {
            gwt_core::process_tree::WindowsJobObject::attach_running(pid)
                .map(|job| Self { job: Some(job) })
                .map_err(|error| format!("Windows Job attach failed for child {pid}: {error}"))
        }

        pub fn apply_resource_policy(
            &mut self,
            pid: u32,
            policy: ProcessResourcePolicy,
        ) -> Result<(), String> {
            let priority = match policy.priority {
                ProcessPriority::Normal => gwt_core::process_tree::WindowsProcessPriority::Normal,
                ProcessPriority::BelowNormal => {
                    gwt_core::process_tree::WindowsProcessPriority::BelowNormal
                }
                ProcessPriority::Idle => gwt_core::process_tree::WindowsProcessPriority::Idle,
            };
            self.job
                .as_ref()
                .ok_or_else(|| "Windows Job is unavailable".to_string())?
                .apply_running_policy(pid, priority, policy.cpu_limit_percent)
                .map_err(|error| format!("Windows Job policy failed for child {pid}: {error}"))
        }

        /// Synchronously terminate every process in the group.
        ///
        /// Idempotent: subsequent calls (including via `Drop`) become no-ops.
        pub fn terminate(&mut self) {
            if let Some(mut job) = self.job.take() {
                let _ = job.terminate();
            }
        }
    }

    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            self.terminate();
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::super::{ProcessPriority, ProcessResourcePolicy};
    use nix::{
        errno::Errno,
        sys::signal::{killpg, Signal},
        unistd::Pid,
    };

    #[derive(Default)]
    pub struct ProcessGroup {
        pgid: Option<Pid>,
    }

    impl ProcessGroup {
        pub fn attach(pid: u32) -> Result<Self, String> {
            // portable_pty spawns each child in its own session via setsid,
            // so the child's pid is also its process group id.
            Ok(Self {
                pgid: Some(Pid::from_raw(pid as i32)),
            })
        }

        pub fn apply_resource_policy(
            &mut self,
            pid: u32,
            policy: ProcessResourcePolicy,
        ) -> Result<(), String> {
            let requested_nice = match policy.priority {
                ProcessPriority::Normal => 0,
                ProcessPriority::BelowNormal => 10,
                ProcessPriority::Idle => 19,
            };
            let inherited_nice = process_nice(pid)?;
            let nice = inherited_nice.max(requested_nice);
            // SAFETY: the PID was returned by the successful PTY spawn and is
            // blocked behind the trusted start gate handshake.
            if unsafe { libc::setpriority(libc::PRIO_PROCESS, pid, nice) } != 0 {
                return Err(format!(
                    "setpriority({pid}, {nice}) failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let observed = process_nice(pid)?;
            if observed != nice {
                return Err(format!(
                    "setpriority({pid}, {nice}) did not stick: observed {observed}"
                ));
            }
            Ok(())
        }

        /// Signal every process in the group without waiting for reap.
        ///
        /// Idempotent: subsequent calls (including via `Drop`) become no-ops.
        /// Issue #3705: SIGKILL is sent immediately. A SIGTERM-then-sleep-then-
        /// SIGKILL sequence blocked the GUI event loop for 100ms per live PTY
        /// close and serialized `pane.*` behind it.
        pub fn terminate(&mut self) {
            let Some(pgid) = self.pgid.take() else {
                return;
            };
            match killpg(pgid, Signal::SIGKILL) {
                Ok(()) | Err(Errno::ESRCH) => {}
                Err(error) => tracing::debug!(?pgid, %error, "killpg SIGKILL failed"),
            }
        }
    }

    fn process_nice(pid: u32) -> Result<i32, String> {
        // `getpriority` may legitimately return -1, so errno must be cleared
        // and inspected separately instead of treating -1 as an unconditional
        // error sentinel.
        Errno::clear();
        let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, pid) };
        let errno = Errno::last_raw();
        if nice == -1 && errno != 0 {
            return Err(format!(
                "getpriority({pid}) failed: {}",
                std::io::Error::from_raw_os_error(errno)
            ));
        }
        Ok(nice)
    }

    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            self.terminate();
        }
    }
}

pub use imp::ProcessGroup;

#[cfg(test)]
mod tests {
    #[test]
    fn windows_process_group_reuses_shared_job_owner() {
        let source = include_str!("process_group.rs");
        assert!(source.contains("WindowsJobObject::attach_running(pid)"));
        assert!(source.contains("pub fn attach(pid: u32) -> Result<Self, String>"));
        assert!(
            !source.contains("Windows Job attach failed\");\n                    Self::default()")
        );
        assert!(!source.contains(concat!("Create", "JobObjectW")));
        assert!(!source.contains(concat!("AssignProcess", "ToJobObject")));
    }
}
