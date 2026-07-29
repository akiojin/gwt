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
            Foundation::{CloseHandle, ERROR_NO_MORE_FILES, HANDLE},
            System::{
                Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD,
                    THREADENTRY32,
                },
                JobObjects::{
                    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
                    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
                    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                },
                Threading::{
                    OpenProcess, OpenThread, ResumeThread, PROCESS_SET_QUOTA, PROCESS_TERMINATE,
                    THREAD_SUSPEND_RESUME,
                },
            },
        },
    };

    use super::WINDOWS_HIDDEN_SUSPENDED_CREATION_FLAGS;

    #[derive(Debug)]
    pub enum WindowsJobError {
        Operation {
            operation: &'static str,
            source: WindowsError,
        },
        PrimaryThreadCount {
            process_id: u32,
        },
        PrimaryThreadNotSuspended {
            process_id: u32,
        },
    }

    impl std::fmt::Display for WindowsJobError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Operation { operation, .. } => write!(formatter, "{operation} failed"),
                Self::PrimaryThreadCount { process_id } => write!(
                    formatter,
                    "suspended process {process_id} did not expose exactly one primary thread"
                ),
                Self::PrimaryThreadNotSuspended { process_id } => write!(
                    formatter,
                    "primary thread for suspended process {process_id} was not suspended"
                ),
            }
        }
    }

    impl std::error::Error for WindowsJobError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Self::Operation { source, .. } => Some(source),
                Self::PrimaryThreadCount { .. } | Self::PrimaryThreadNotSuspended { .. } => None,
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

    fn resume_suspended_process_threads(process_id: u32) -> Result<(), WindowsJobError> {
        // SAFETY: the returned snapshot handle is owned by this function.
        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
            .map(ScopedHandle)
            .map_err(|source| operation_error("CreateToolhelp32Snapshot", source))?;
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..THREADENTRY32::default()
        };
        // SAFETY: `entry` has the required size and remains valid throughout
        // enumeration.
        unsafe { Thread32First(snapshot.0, &mut entry) }
            .map_err(|source| operation_error("Thread32First", source))?;
        let mut primary_threads = Vec::new();
        loop {
            if entry.th32OwnerProcessID == process_id {
                // SAFETY: OpenThread returns a new owned handle for this
                // snapshot entry.
                let thread =
                    unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) }
                        .map(ScopedHandle)
                        .map_err(|source| operation_error("OpenThread", source))?;
                primary_threads.push(thread);
            }
            // SAFETY: the snapshot and entry remain live and correctly sized.
            if let Err(source) = unsafe { Thread32Next(snapshot.0, &mut entry) } {
                if source.code() == HRESULT::from_win32(ERROR_NO_MORE_FILES.0) {
                    break;
                }
                return Err(operation_error("Thread32Next", source));
            }
        }
        if primary_threads.len() != 1 {
            return Err(WindowsJobError::PrimaryThreadCount { process_id });
        }
        // SAFETY: CREATE_SUSPENDED guarantees this sole primary thread has a
        // positive suspend count until this call.
        let previous_count = unsafe { ResumeThread(primary_threads[0].0) };
        if previous_count == u32::MAX {
            return Err(operation_error("ResumeThread", WindowsError::from_thread()));
        }
        if previous_count == 0 {
            return Err(WindowsJobError::PrimaryThreadNotSuspended { process_id });
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
}
