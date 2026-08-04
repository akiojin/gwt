//! Shared operating-system process-tree ownership primitives.

/// `CREATE_NO_WINDOW`, retained when a health probe adds suspended creation.
pub const WINDOWS_CREATE_NO_WINDOW: u32 = 0x0800_0000;
/// `CREATE_SUSPENDED`, used to prevent child code from running before Job assignment.
pub const WINDOWS_CREATE_SUSPENDED: u32 = 0x0000_0004;
/// Creation flags for a hidden Windows child whose Job is assigned before execution.
pub const WINDOWS_HIDDEN_SUSPENDED_CREATION_FLAGS: u32 =
    WINDOWS_CREATE_NO_WINDOW | WINDOWS_CREATE_SUSPENDED;

#[cfg(windows)]
mod windows_job {
    use std::process::Command;

    use windows::{
        core::{Error as WindowsError, HRESULT},
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            System::{
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                    QueryInformationJobObject, SetInformationJobObject,
                    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                },
                Threading::{
                    OpenProcess, PROCESS_SET_QUOTA, PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE,
                },
            },
        },
    };

    use super::WINDOWS_HIDDEN_SUSPENDED_CREATION_FLAGS;

    // Resumes every thread of a process from a process handle alone. Not part
    // of the Win32 metadata the `windows` crate is generated from, so it is
    // declared here; ntdll exports it on every supported Windows release.
    #[link(name = "ntdll")]
    extern "system" {
        fn NtResumeProcess(process: HANDLE) -> i32;
    }

    #[derive(Debug)]
    pub enum WindowsJobError {
        Operation {
            operation: &'static str,
            source: WindowsError,
        },
    }

    impl std::fmt::Display for WindowsJobError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Operation { operation, .. } => write!(formatter, "{operation} failed"),
            }
        }
    }

    impl std::error::Error for WindowsJobError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Self::Operation { source, .. } => Some(source),
            }
        }
    }

    struct ScopedHandle(HANDLE);

    impl Drop for ScopedHandle {
        fn drop(&mut self) {
            // SAFETY: this wrapper is created only for an owned Win32 handle
            // and is dropped exactly once.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    /// Owner for a Windows Job Object configured to kill all members when its
    /// final handle closes.
    pub struct WindowsJobObject {
        handle: Option<HANDLE>,
    }

    // Job handles are kernel-owned values. Access to the optional owner is
    // exclusive for mutation, and CloseHandle is safe from any thread.
    unsafe impl Send for WindowsJobObject {}
    unsafe impl Sync for WindowsJobObject {}

    impl WindowsJobObject {
        pub fn new() -> Result<Self, WindowsJobError> {
            // SAFETY: null name/security create a private Job Object owned by
            // the returned handle.
            let job = unsafe { CreateJobObjectW(None, None) }
                .map_err(|source| operation_error("CreateJobObjectW", source))?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let info_size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
            // SAFETY: `info` is the exact structure required by the selected
            // information class and remains alive for the duration of the call.
            if let Err(source) = unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &info as *const _ as _,
                    info_size,
                )
            } {
                // SAFETY: `job` is owned by this function on the error path.
                unsafe {
                    let _ = CloseHandle(job);
                }
                return Err(operation_error("SetInformationJobObject", source));
            }
            Ok(Self { handle: Some(job) })
        }

        /// Configure a command to start suspended and without a console
        /// window. The suspended primary thread cannot create descendants
        /// before [`Self::assign_and_resume`] installs Job ownership.
        pub fn configure_suspended(command: &mut Command) {
            use std::os::windows::process::CommandExt;

            command.creation_flags(WINDOWS_HIDDEN_SUSPENDED_CREATION_FLAGS);
        }

        /// Create a kill-on-close Job and attach an already running process.
        ///
        /// This compatibility path is for APIs such as `portable-pty` that do
        /// not expose a pre-execution creation hook. New direct spawn paths
        /// should use `new` + `configure_suspended` + `assign_and_resume`.
        pub fn attach_running(process_id: u32) -> Result<Self, WindowsJobError> {
            let mut job = Self::new()?;
            job.assign_process(process_id)?;
            Ok(job)
        }

        /// Assign a newly-created suspended process, then resume its sole
        /// primary thread. Assignment deliberately precedes all resume work.
        pub fn assign_and_resume(&mut self, process_id: u32) -> Result<(), WindowsJobError> {
            self.assign_process(process_id)?;
            resume_suspended_process_threads(process_id)
        }

        /// Close the Job handle synchronously. With
        /// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, this terminates every member
        /// and descendant still assigned to the Job.
        pub fn terminate(&mut self) -> bool {
            self.handle.take().is_none_or(|job| {
                // SAFETY: `job` is owned by `self` and removed before close,
                // making repeated termination and Drop idempotent.
                unsafe { CloseHandle(job).is_ok() }
            })
        }

        /// Relinquish this process' Job handle without terminating processes
        /// that remain in the Job.
        ///
        /// The existing extended-limit structure is queried and written back
        /// verbatim except for `KILL_ON_JOB_CLOSE`. This is deliberately a
        /// consuming operation: until the updated limits and handle close both
        /// succeed, `self` retains ownership so its armed `Drop` remains the
        /// failure-path safety net.
        pub fn release_without_termination(mut self) -> Result<(), WindowsJobError> {
            let job = self.handle.expect("live Windows Job handle");
            let mut original = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            let info_size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
            // SAFETY: `original` is the exact buffer required by the selected
            // information class and is writable for the duration of the call.
            unsafe {
                QueryInformationJobObject(
                    Some(job),
                    JobObjectExtendedLimitInformation,
                    &mut original as *mut _ as _,
                    info_size,
                    None,
                )
            }
            .map_err(|source| operation_error("QueryInformationJobObject", source))?;

            let mut released = original;
            released.BasicLimitInformation.LimitFlags &= !JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            // SAFETY: `released` is the exact structure required by the
            // selected information class and remains live for this call.
            unsafe {
                SetInformationJobObject(
                    job,
                    JobObjectExtendedLimitInformation,
                    &released as *const _ as _,
                    info_size,
                )
            }
            .map_err(|source| operation_error("SetInformationJobObject", source))?;

            // Remove ownership only after CloseHandle succeeds. If closing
            // unexpectedly fails, restore the armed limits before Drop makes
            // one final close attempt.
            if let Err(source) = unsafe { CloseHandle(job) } {
                // SAFETY: best-effort restoration uses the previously queried
                // exact structure; failure still leaves the owned handle for
                // Drop to close.
                let _ = unsafe {
                    SetInformationJobObject(
                        job,
                        JobObjectExtendedLimitInformation,
                        &original as *const _ as _,
                        info_size,
                    )
                };
                return Err(operation_error("CloseHandle", source));
            }
            self.handle = None;
            Ok(())
        }

        fn assign_process(&mut self, process_id: u32) -> Result<(), WindowsJobError> {
            let job = self.handle.expect("live Windows Job handle");
            // SAFETY: OpenProcess returns a new owned handle for the exact PID.
            let process =
                unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, process_id) }
                    .map(ScopedHandle)
                    .map_err(|source| operation_error("OpenProcess", source))?;
            // SAFETY: both handles are live for this call. The Job retains
            // process membership after the temporary process handle closes.
            unsafe { AssignProcessToJobObject(job, process.0) }
                .map_err(|source| operation_error("AssignProcessToJobObject", source))
        }
    }

    impl Drop for WindowsJobObject {
        fn drop(&mut self) {
            let _ = self.terminate();
        }
    }

    /// Resume a process created with `CREATE_SUSPENDED`.
    ///
    /// Cost here is on the critical path of every deadline-bounded spawn, so
    /// the work must be scoped to the target process. `NtResumeProcess` takes
    /// a process handle and needs no thread lookup. The ToolHelp alternative
    /// cannot be scoped — `CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, ..)`
    /// ignores its process-id argument and always snapshots every thread on
    /// the machine, so its cost tracks total system thread count (~100ms at
    /// ~8800 threads) and is paid once per spawn.
    fn resume_suspended_process_threads(process_id: u32) -> Result<(), WindowsJobError> {
        // SAFETY: OpenProcess returns a new owned handle for the exact PID.
        let process = unsafe { OpenProcess(PROCESS_SUSPEND_RESUME, false, process_id) }
            .map(ScopedHandle)
            .map_err(|source| operation_error("OpenProcess", source))?;
        // SAFETY: `process` is live for this call and was opened with the
        // PROCESS_SUSPEND_RESUME access NtResumeProcess requires.
        let status = unsafe { NtResumeProcess(process.0) };
        if status < 0 {
            return Err(operation_error(
                "NtResumeProcess",
                WindowsError::from_hresult(HRESULT(status)),
            ));
        }
        Ok(())
    }

    fn operation_error(operation: &'static str, source: WindowsError) -> WindowsJobError {
        WindowsJobError::Operation { operation, source }
    }
}

#[cfg(windows)]
pub use windows_job::{WindowsJobError, WindowsJobObject};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suspended_job_spawn_preserves_hidden_window_flag() {
        assert_eq!(
            WINDOWS_HIDDEN_SUSPENDED_CREATION_FLAGS & WINDOWS_CREATE_NO_WINDOW,
            WINDOWS_CREATE_NO_WINDOW,
        );
        assert_eq!(
            WINDOWS_HIDDEN_SUSPENDED_CREATION_FLAGS & WINDOWS_CREATE_SUSPENDED,
            WINDOWS_CREATE_SUSPENDED,
        );
    }

    #[test]
    fn windows_job_contract_assigns_before_resume_without_shell_fallback() {
        let source = include_str!("process_tree.rs");
        let assign_and_resume = source
            .split("pub fn assign_and_resume")
            .nth(1)
            .and_then(|tail| tail.split("/// Close the Job handle").next())
            .expect("assign_and_resume body");
        let assign = assign_and_resume
            .find("self.assign_process(process_id)")
            .expect("assignment call");
        let resume = assign_and_resume
            .find("resume_suspended_process_threads(process_id)")
            .expect("resume call");

        assert!(assign < resume, "Job assignment must precede thread resume");
        assert!(!source.contains(concat!("task", "kill")));
    }

    #[test]
    fn windows_job_resume_stays_scoped_to_the_target_process() {
        let source = include_str!("process_tree.rs");
        let resume = source
            .split("fn resume_suspended_process_threads")
            .nth(1)
            .and_then(|tail| tail.split("fn operation_error").next())
            .expect("resume_suspended_process_threads body");

        assert!(resume.contains("NtResumeProcess"));
        // A system-wide thread snapshot costs ~100ms per spawn and pushes
        // short deadline-bounded commands past their budget.
        assert!(!resume.contains(concat!("CreateToolhelp32", "Snapshot")));
        assert!(!resume.contains(concat!("Thread32", "First")));
    }

    #[test]
    fn windows_job_release_preserves_existing_extended_limits() {
        let source = include_str!("process_tree.rs");
        let release = source
            .split("pub fn release_without_termination")
            .nth(1)
            .and_then(|tail| tail.split("fn assign_process").next())
            .expect("release_without_termination body");

        assert!(release.contains("QueryInformationJobObject"));
        assert!(release.contains("let mut released = original"));
        assert!(release.contains("&= !JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE"));
        assert!(release.contains("SetInformationJobObject"));
        assert!(
            release.find("CloseHandle(job)").unwrap() < release.find("self.handle = None").unwrap()
        );
    }
}
