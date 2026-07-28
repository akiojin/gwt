//! Platform-specific process group management for PTY children.
//!
//! - Windows: Wraps the child in a Job Object with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so that closing the job handle
//!   terminates the child and every descendant it spawned.
//! - Unix: `portable_pty` already places the child in its own session
//!   (setsid), so the child's pid is also its process group id. On drop we
//!   send `SIGTERM` then `SIGKILL` to the group via `killpg`.

#[cfg(windows)]
mod imp {
    #[derive(Default)]
    pub struct ProcessGroup {
        job: Option<gwt_core::process_tree::WindowsJobObject>,
    }

    impl ProcessGroup {
        pub fn attach(pid: u32) -> Self {
            match gwt_core::process_tree::WindowsJobObject::attach_running(pid) {
                Ok(job) => Self { job: Some(job) },
                Err(error) => {
                    tracing::debug!(pid, %error, "Windows Job attach failed");
                    Self::default()
                }
            }
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
    use std::{thread, time::Duration};

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
        pub fn attach(pid: u32) -> Self {
            // portable_pty spawns each child in its own session via setsid,
            // so the child's pid is also its process group id.
            Self {
                pgid: Some(Pid::from_raw(pid as i32)),
            }
        }

        /// Synchronously signal every process in the group.
        ///
        /// Idempotent: subsequent calls (including via `Drop`) become no-ops.
        pub fn terminate(&mut self) {
            let Some(pgid) = self.pgid.take() else {
                return;
            };
            // SIGTERM first for clean shutdown, then SIGKILL as the safety net.
            match killpg(pgid, Signal::SIGTERM) {
                Ok(()) | Err(Errno::ESRCH) => {}
                Err(error) => tracing::debug!(?pgid, %error, "killpg SIGTERM failed"),
            }
            thread::sleep(Duration::from_millis(100));
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
        assert!(!source.contains(concat!("Create", "JobObjectW")));
        assert!(!source.contains(concat!("AssignProcess", "ToJobObject")));
    }
}
