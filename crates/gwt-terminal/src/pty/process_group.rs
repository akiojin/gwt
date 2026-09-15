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
        /// The policy's CPU cap, restored once this tree stops holding the
        /// verification lease (Issue #4405).
        cpu_cap: Option<u8>,
        cap_lifted: bool,
    }

    impl ProcessGroup {
        pub fn attach(pid: u32) -> Result<Self, String> {
            gwt_core::process_tree::WindowsJobObject::attach_running(pid)
                .map(|job| Self {
                    job: Some(job),
                    ..Self::default()
                })
                .map_err(|error| format!("Windows Job attach failed for child {pid}: {error}"))
        }

        /// Issue #4405 AC-2: the verification lease serializes heavy
        /// verification host-wide, so its holder must not run at the
        /// per-agent CPU share. Lift this Job's cap while `holder_pid` runs
        /// inside it and restore the policy cap once it does not. Nothing
        /// leaves the Job, so kill-on-close containment is untouched. Returns
        /// whether the cap changed.
        pub fn relieve_cap_for_lease_holder(
            &mut self,
            holder_pid: Option<u32>,
        ) -> Result<bool, String> {
            let (Some(cap), Some(job)) = (self.cpu_cap, self.job.as_mut()) else {
                return Ok(false);
            };
            // A holder that exited between the lease read and this check is
            // no longer anyone's to relieve.
            let holds = holder_pid.is_some_and(|pid| job.contains_process(pid).unwrap_or(false));
            if holds == self.cap_lifted {
                return Ok(false);
            }
            job.set_cpu_rate_hard_cap(if holds { 100 } else { cap })
                .map_err(|error| format!("configure Job CPU hard cap: {error}"))?;
            self.cap_lifted = holds;
            Ok(true)
        }

        #[cfg(test)]
        pub(super) fn cpu_cap_percent(&self) -> Option<u8> {
            self.job
                .as_ref()?
                .cpu_rate_hard_cap_percent()
                .ok()
                .flatten()
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
                self.cpu_cap = Some(percent);
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

        /// Unix has no tree-wide CPU cap to lift (Issue #4405).
        pub fn relieve_cap_for_lease_holder(
            &mut self,
            _holder_pid: Option<u32>,
        ) -> Result<bool, String> {
            Ok(false)
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

    /// Issue #4405 AC-2: the cap lifts only while the lease holder runs in
    /// this Job, and the policy cap comes back once it does not.
    #[cfg(windows)]
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the test needs a plain paused child to own a Job for"
    )]
    fn the_cpu_cap_lifts_only_while_the_lease_holder_runs_in_the_job() {
        use std::process::{Command, Stdio};

        use super::super::{ProcessPolicy, ProcessPriority};

        let mut child = Command::new("cmd")
            .args(["/C", "pause"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn paused child");
        let pid = child.id();
        let mut group = super::imp::ProcessGroup::attach(pid).expect("attach Job");
        group
            .apply_policy(
                pid,
                ProcessPolicy {
                    priority: ProcessPriority::BelowNormal,
                    cpu_limit_percent: Some(25),
                },
            )
            .expect("apply policy");
        assert_eq!(group.cpu_cap_percent(), Some(25));

        let outsider = std::process::id();
        assert!(!group.relieve_cap_for_lease_holder(Some(outsider)).unwrap());
        assert_eq!(group.cpu_cap_percent(), Some(25), "a holder elsewhere");
        // A holder that already exited is simply not here: no error, no change.
        assert!(!group
            .relieve_cap_for_lease_holder(Some(u32::MAX - 3))
            .unwrap());
        assert_eq!(group.cpu_cap_percent(), Some(25), "a vanished holder");

        assert!(group.relieve_cap_for_lease_holder(Some(pid)).unwrap());
        assert_eq!(group.cpu_cap_percent(), Some(100), "the holder is here");
        assert!(!group.relieve_cap_for_lease_holder(Some(pid)).unwrap());

        assert!(group.relieve_cap_for_lease_holder(None).unwrap());
        assert_eq!(group.cpu_cap_percent(), Some(25), "the lease is free");

        group.terminate();
        let _ = child.wait();
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
