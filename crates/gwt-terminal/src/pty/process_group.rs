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

        /// Set the nice value of the whole process group (portable_pty runs the
        /// child under `setsid`, so the group id is the child pid). fork/exec
        /// descendants inherit it. `PRIO_PGRP` is deliberate: on Linux
        /// `PRIO_PROCESS` only reaches the thread whose id equals `pid`, and a
        /// helper that execs the target from another thread would keep nice 0.
        /// `cpu_limit_percent` has no tree-wide Unix equivalent and is ignored.
        pub fn apply_policy(&mut self, pid: u32, policy: ProcessPolicy) -> Result<(), String> {
            apply_group_nice(pid, policy.priority.unix_nice(), set_group_nice)
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

    /// Describe a rejected renice with the platform reason. Issue #3942:
    /// `setpriority` returns EPERM whenever the caller may not renice the
    /// target group, so the reason has to reach the launch route's warning.
    pub(super) fn apply_group_nice(
        pid: u32,
        nice: i32,
        set: impl Fn(u32, i32) -> Result<(), std::io::Error>,
    ) -> Result<(), String> {
        set(pid, nice).map_err(|error| format!("setpriority(pgrp {pid}, nice {nice}): {error}"))
    }

    fn set_group_nice(pid: u32, nice: i32) -> Result<(), std::io::Error> {
        // SAFETY: setpriority has no memory-safety preconditions.
        let status = unsafe { libc::setpriority(libc::PRIO_PGRP as _, pid as libc::id_t, nice) };
        if status != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

pub use imp::ProcessGroup;

#[cfg(test)]
mod tests {
    /// Issue #3942: on hosts where the launcher may not renice the target's
    /// group, `setpriority` fails with EPERM. The failure must keep the
    /// platform reason so the launch route can warn with something actionable.
    #[cfg(unix)]
    #[test]
    fn setpriority_eperm_is_reported_with_the_platform_reason() {
        let error = super::imp::apply_group_nice(4242, 10, |_, _| {
            Err(std::io::Error::from_raw_os_error(libc::EPERM))
        })
        .expect_err("EPERM must surface as a policy error");
        assert!(error.contains("setpriority(pgrp 4242, nice 10)"), "{error}");
        assert!(error.contains("Operation not permitted"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn applied_group_nice_reports_success() {
        assert_eq!(
            super::imp::apply_group_nice(4242, 10, |_, _| Ok(())),
            Ok(())
        );
    }

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
