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
    use windows::Win32::{
        Foundation::{CloseHandle, HANDLE},
        System::JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
            SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        },
        System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
    };

    #[derive(Default)]
    pub struct ProcessGroup {
        job: Option<HANDLE>,
    }

    // A Win32 Job Object HANDLE is an opaque pointer safe to share across
    // threads; our access is serialized by Drop anyway.
    unsafe impl Send for ProcessGroup {}
    unsafe impl Sync for ProcessGroup {}

    impl ProcessGroup {
        pub fn attach(pid: u32) -> Self {
            unsafe {
                let job = match CreateJobObjectW(None, None) {
                    Ok(h) => h,
                    Err(error) => {
                        tracing::debug!(%error, "CreateJobObjectW failed");
                        return Self::default();
                    }
                };

                let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
                info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
                let info_size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
                if let Err(error) = SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as _,
                    info_size,
                ) {
                    tracing::debug!(%error, "SetInformationJobObject failed");
                    let _ = CloseHandle(job);
                    return Self::default();
                }

                let proc = match OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) {
                    Ok(h) => h,
                    Err(error) => {
                        tracing::debug!(pid, %error, "OpenProcess failed");
                        let _ = CloseHandle(job);
                        return Self::default();
                    }
                };

                let assign = AssignProcessToJobObject(job, proc);
                let _ = CloseHandle(proc);
                if let Err(error) = assign {
                    tracing::debug!(pid, %error, "AssignProcessToJobObject failed");
                    let _ = CloseHandle(job);
                    return Self::default();
                }

                Self { job: Some(job) }
            }
        }

        /// Synchronously terminate every process in the group.
        ///
        /// Idempotent: subsequent calls (including via `Drop`) become no-ops.
        pub fn terminate(&mut self) {
            if let Some(job) = self.job.take() {
                // Closing the last handle to a Job configured with
                // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE terminates every process
                // still assigned to it.
                unsafe {
                    let _ = CloseHandle(job);
                }
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
