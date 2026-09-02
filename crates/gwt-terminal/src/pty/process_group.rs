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
    use super::super::ProcessPolicy;

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

        /// Lower the tree root's priority class and, when requested, cap the
        /// Job's CPU rate. Kill-on-close remains armed either way.
        pub fn apply_policy(&mut self, pid: u32, policy: ProcessPolicy) -> Result<(), String> {
            gwt_core::process_tree::set_process_priority_class(
                pid,
                policy.priority.windows_priority_class(),
            )
            .map_err(|error| format!("set priority class for child {pid}: {error}"))?;
            if let Some(percent) = policy.cpu_limit_percent {
                let job = self
                    .job
                    .as_mut()
                    .ok_or_else(|| format!("Windows Job is not attached for child {pid}"))?;
                job.set_cpu_rate_hard_cap(percent)
                    .map_err(|error| format!("configure Job CPU hard cap: {error}"))?;
            }
            Ok(())
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
    use nix::{
        errno::Errno,
        sys::signal::{killpg, Signal},
        unistd::Pid,
    };

    use super::super::ProcessPolicy;

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

        /// Set the tree root's nice value. fork/exec descendants inherit it.
        /// `cpu_limit_percent` has no tree-wide Unix equivalent and is ignored.
        pub fn apply_policy(&mut self, pid: u32, policy: ProcessPolicy) -> Result<(), String> {
            let nice = policy.priority.unix_nice();
            // SAFETY: setpriority has no memory-safety preconditions.
            let status =
                unsafe { libc::setpriority(libc::PRIO_PROCESS as _, pid as libc::id_t, nice) };
            if status != 0 {
                return Err(format!(
                    "setpriority(pid {pid}, nice {nice}): {}",
                    std::io::Error::last_os_error()
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
