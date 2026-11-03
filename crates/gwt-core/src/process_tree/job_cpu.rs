//! Kernel CPU-cap observations for gwt-owned pane Jobs (Issue #5816).
//!
//! A verification child has its own uncapped containment Job. Querying only
//! that immediate Job hides capped ancestors, so panes publish UUID Job names
//! in the shared runtime directory. Observers open query-only handles briefly
//! and retain no handles between samples, preserving pane-close containment.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobCpuCap {
    pub job_name: String,
    pub owner_pid: u32,
    pub hard_cap_percent: Option<u8>,
}

/// The immediate native Job is separate evidence from the tracked pane chain.
/// In particular, an old GUI can own an unregistered, capped ancestor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentJobCpu {
    pub in_job: bool,
    pub hard_cap_percent: Option<u8>,
}

/// Read every tracked pane Job containing `pid`, including nested ancestors.
/// Missing names are exited Jobs; unreadable observations are errors, never
/// evidence of an uncapped environment. Non-Windows platforms have no Job cap.
pub fn observe(pid: u32) -> Result<Vec<JobCpuCap>, String> {
    #[cfg(windows)]
    {
        windows::observe(pid)
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Ok(Vec::new())
    }
}

/// Query the caller's immediate native Job without assuming it is registered.
pub fn observe_current() -> Result<CurrentJobCpu, String> {
    #[cfg(windows)]
    {
        windows::observe_current()
    }
    #[cfg(not(windows))]
    {
        Ok(CurrentJobCpu {
            in_job: false,
            hard_cap_percent: None,
        })
    }
}

#[cfg(windows)]
pub(super) use windows::{hard_cap_percent, register};

#[cfg(windows)]
mod windows {
    use std::{fs, io, path::PathBuf};

    use ::windows::{
        core::{Owned, HRESULT, PCWSTR},
        Win32::{
            Foundation::{ERROR_FILE_NOT_FOUND, ERROR_INVALID_PARAMETER, HANDLE},
            System::{
                JobObjects::{
                    IsProcessInJob, JobObjectCpuRateControlInformation, OpenJobObjectW,
                    QueryInformationJobObject, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION,
                    JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                },
                Threading::{GetCurrentProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
            },
        },
    };

    use super::{CurrentJobCpu, JobCpuCap};

    // Win32's query-only Job access right. Its metadata constant lives in the
    // unrelated SystemServices feature; OpenJobObjectW accepts the raw mask.
    const JOB_OBJECT_QUERY: u32 = 0x0004;

    #[derive(serde::Serialize, serde::Deserialize)]
    struct PaneJob {
        name: String,
        owner_pid: u32,
    }

    fn registry_dir() -> PathBuf {
        crate::paths::gwt_runtime_dir().join("windows-pane-jobs")
    }

    pub(in crate::process_tree) fn register(name: &str) -> io::Result<PathBuf> {
        let directory = registry_dir();
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!("{}.json", uuid::Uuid::new_v4()));
        let descriptor = PaneJob {
            name: name.to_string(),
            owner_pid: std::process::id(),
        };
        let bytes = serde_json::to_vec(&descriptor).map_err(io::Error::other)?;
        crate::atomic_file::write_atomic(&path, &bytes)?;
        Ok(path)
    }

    pub(super) fn observe(pid: u32) -> Result<Vec<JobCpuCap>, String> {
        let entries = match fs::read_dir(registry_dir()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(format!("read pane Job registry: {error}")),
        };
        // SAFETY: this opens an owned query-only handle to the supplied PID.
        let process = match unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) } {
            Ok(handle) => unsafe { Owned::new(handle) },
            // Process tables race normal child exits. Permission failures and
            // other unknown observations still fail closed.
            Err(error) if error.code() == HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => {
                return Ok(Vec::new());
            }
            Err(error) => return Err(format!("query process {pid}: {error}")),
        };
        let mut observed = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| format!("read pane Job entry: {error}"))?;
            if entry
                .path()
                .extension()
                .is_none_or(|extension| extension != "json")
            {
                continue;
            }
            let bytes = match fs::read(entry.path()) {
                Ok(bytes) => bytes,
                Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                Err(error) => return Err(format!("read pane Job descriptor: {error}")),
            };
            let descriptor: PaneJob = serde_json::from_slice(&bytes)
                .map_err(|error| format!("parse pane Job descriptor: {error}"))?;
            let name: Vec<_> = descriptor.name.encode_utf16().chain(Some(0)).collect();
            // SAFETY: the name is NUL-terminated and alive for this call. The
            // observer requests no ability to change or terminate the Job.
            let job = match unsafe {
                OpenJobObjectW(JOB_OBJECT_QUERY, false, PCWSTR(name.as_ptr()))
            } {
                Ok(handle) => unsafe { Owned::new(handle) },
                Err(error) if error.code() == HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0) => {
                    continue
                }
                Err(error) => return Err(format!("open pane Job {}: {error}", descriptor.name)),
            };
            let mut contains = ::windows::core::BOOL::default();
            // SAFETY: both query handles and the output buffer remain live.
            unsafe { IsProcessInJob(*process, Some(*job), &mut contains) }
                .map_err(|error| format!("query pane Job membership: {error}"))?;
            if contains.as_bool() {
                let cap = hard_cap_percent(Some(*job))
                    .map_err(|error| format!("query pane Job CPU cap: {error}"))?;
                observed.push(JobCpuCap {
                    job_name: descriptor.name,
                    owner_pid: descriptor.owner_pid,
                    hard_cap_percent: cap,
                });
            }
        }
        observed.sort_by(|left, right| left.job_name.cmp(&right.job_name));
        Ok(observed)
    }

    pub(super) fn observe_current() -> Result<CurrentJobCpu, String> {
        let mut in_job = ::windows::core::BOOL::default();
        // SAFETY: the current-process pseudo-handle and output are valid.
        unsafe { IsProcessInJob(GetCurrentProcess(), None, &mut in_job) }
            .map_err(|error| format!("query current Job membership: {error}"))?;
        let hard_cap_percent = if in_job.as_bool() {
            hard_cap_percent(None).map_err(|error| format!("query current Job CPU cap: {error}"))?
        } else {
            None
        };
        Ok(CurrentJobCpu {
            in_job: in_job.as_bool(),
            hard_cap_percent,
        })
    }

    pub(in crate::process_tree) fn hard_cap_percent(
        job: Option<HANDLE>,
    ) -> ::windows::core::Result<Option<u8>> {
        let mut info = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION::default();
        // SAFETY: the exact writable buffer remains live throughout the query.
        unsafe {
            QueryInformationJobObject(
                job,
                JobObjectCpuRateControlInformation,
                &mut info as *mut _ as _,
                std::mem::size_of_val(&info) as u32,
                None,
            )
        }?;
        let required =
            JOB_OBJECT_CPU_RATE_CONTROL_ENABLE.0 | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP.0;
        if info.ControlFlags.0 & required != required {
            return Ok(None);
        }
        // SAFETY: HARD_CAP selects the CpuRate member of the union.
        Ok(Some(
            (unsafe { info.Anonymous.CpuRate } / 100).min(100) as u8
        ))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::process::{Command, Stdio};

    use super::*;
    use crate::{process_tree::WindowsJobObject, test_support::ScopedGwtHome};

    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "paused child fixtures exercise native Job membership"
    )]
    fn tracked_jobs_observe_nested_caps_and_remove_descriptors() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let mut child = Command::new("cmd")
            .args(["/C", "pause"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut outer = WindowsJobObject::attach_running(pid).unwrap();
        outer.set_cpu_rate_hard_cap(40).unwrap();
        let mut inner = WindowsJobObject::attach_running(pid).unwrap();
        inner.set_cpu_rate_hard_cap(25).unwrap();
        let caps = observe(pid).unwrap();
        assert_eq!(caps.len(), 2);
        assert!(caps.iter().all(|cap| cap.owner_pid == std::process::id()));
        assert!(caps.iter().any(|cap| cap.hard_cap_percent == Some(40)));
        assert!(caps.iter().any(|cap| cap.hard_cap_percent == Some(25)));
        assert!(observe(std::process::id()).unwrap().is_empty());
        inner.disable_cpu_rate_control().unwrap();
        outer.disable_cpu_rate_control().unwrap();
        assert!(observe(pid)
            .unwrap()
            .iter()
            .all(|cap| cap.hard_cap_percent.is_none()));
        assert!(inner.kill_on_close_enabled().unwrap());
        assert!(outer.kill_on_close_enabled().unwrap());
        drop(inner);
        drop(outer);
        let _ = child.wait();
        assert_eq!(
            std::fs::read_dir(crate::paths::gwt_runtime_dir().join("windows-pane-jobs"))
                .unwrap()
                .count(),
            0
        );
    }
}
