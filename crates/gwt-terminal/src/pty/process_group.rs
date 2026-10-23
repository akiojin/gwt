//! Platform-specific process group management for PTY children.
//!
//! - Windows: Wraps the child in a Job Object with
//!   `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` so that closing the job handle
//!   terminates the child and every descendant it spawned.
//! - Unix: `portable_pty` places the child in its own session (setsid).
//!   Nonblocking termination signals its original process group; background
//!   confirmation also reclaims other groups in that captured session.

#[cfg(windows)]
mod imp {
    use std::time::{Duration, Instant};

    use super::super::ProcessPolicy;

    #[derive(Default)]
    pub struct ProcessGroup {
        job: Option<gwt_core::process_tree::WindowsJobObject>,
        /// The policy's CPU cap, restored once this tree stops holding the
        /// verification lease (Issue #4405).
        cpu_cap: Option<u8>,
        cap_lifted: bool,
        termination_requested: bool,
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

        /// Issues #4405 / #5816: canonical verification holders must not run
        /// at the per-agent CPU share. Disable this Job's rate control while
        /// any holder runs inside it and restore the policy cap afterwards. Nothing
        /// leaves the Job, so kill-on-close containment is untouched. Returns
        /// whether the cap changed.
        pub fn relieve_cap_for_lease_holders(
            &mut self,
            holder_pids: &[u32],
        ) -> Result<bool, String> {
            let (Some(cap), Some(job)) = (self.cpu_cap, self.job.as_mut()) else {
                return Ok(false);
            };
            // A holder that exited between the lease read and this check is
            // no longer anyone's to relieve.
            let holds = holder_pids
                .iter()
                .any(|pid| job.contains_process(*pid).unwrap_or(false));
            if holds == self.cap_lifted {
                return Ok(false);
            }
            if holds {
                job.disable_cpu_rate_control()
            } else {
                job.set_cpu_rate_hard_cap(cap)
            }
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

        /// Request termination without consuming the captured Job handle.
        pub fn terminate(&mut self) -> Result<(), String> {
            if self.termination_requested {
                return Ok(());
            }
            let job = self
                .job
                .as_mut()
                .ok_or("PTY process Job was not captured")?;
            job.request_termination()
                .map_err(|error| format!("terminate PTY process Job: {error}"))?;
            self.termination_requested = true;
            Ok(())
        }

        /// Keep the exact Job and process handles until every captured member
        /// has signaled completion, beyond the Job's active-count decrement.
        pub fn terminate_and_wait(&mut self, timeout: Duration) -> Result<bool, String> {
            let started = Instant::now();
            self.terminate()?;
            loop {
                let Some(job) = self.job.as_ref() else {
                    return Ok(true);
                };
                if job
                    .termination_confirmed()
                    .map_err(|error| format!("query PTY process Job: {error}"))?
                {
                    self.job = None;
                    return Ok(true);
                }
                let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                    return Ok(false);
                };
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
        }
    }

    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            let _ = self.terminate();
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::time::{Duration, Instant};

    use nix::{
        errno::Errno,
        sys::signal::{killpg, Signal},
        unistd::Pid,
    };

    use super::super::ProcessPolicy;

    #[derive(Default)]
    pub struct ProcessGroup {
        pgid: Option<Pid>,
        session_id: Option<Pid>,
        captured_root: Option<NativeProcess>,
        termination_requested: bool,
        tree_exited: bool,
    }

    impl ProcessGroup {
        pub fn attach(pid: u32) -> Result<Self, String> {
            // portable_pty spawns each child in its own session via setsid,
            // so only this exact PID can be the owned SID. Never adopt a
            // caller/shared session if an unexpected child lacks setsid.
            let pid = i32::try_from(pid)
                .ok()
                .filter(|pid| *pid > 0)
                .ok_or("invalid PTY child PID")?;
            let captured_root = match process_session(pid)? {
                Some(session_id) => {
                    if session_id != pid {
                        return Err(format!("PTY child {pid} does not lead its own session"));
                    }
                    let root = native_process(pid)?;
                    if root.is_some_and(|root| root.session_id != pid) {
                        return Err(format!("PTY child {pid} changed session during attachment"));
                    }
                    root
                }
                None => {
                    if native_process(pid)?.is_some() {
                        return Err(format!("PTY child {pid} reappeared during attachment"));
                    }
                    None
                }
            };
            // A short-lived child may already be reaped. Its SID still comes
            // from portable_pty's setsid contract, never a different live SID.
            let pid = Pid::from_raw(pid);
            Ok(Self {
                pgid: Some(pid),
                session_id: Some(pid),
                captured_root,
                termination_requested: false,
                tree_exited: false,
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

        /// Unix has no tree-wide CPU cap to lift, and no reversible priority
        /// lever to replace it with (Issue #4405 AC-2).
        ///
        /// Measured on the reporting host (macOS 25.5, unprivileged):
        ///
        /// ```text
        /// setpriority(self, 10)  -> 0        raising is always permitted
        /// setpriority(self, 0)   -> EACCES   even back down to where we began
        /// setpriority(child, 0)  -> EACCES   the launcher cannot lower it either
        /// ```
        ///
        /// So neither half of AC-2 is reachable here. The holder's nice cannot
        /// be lowered (that needs root), and suppressing the other panes by
        /// raising theirs is a one-way door: nothing could restore them when
        /// the lease is released, so every pane would decay to nice 19 after
        /// the first verification and stay there for its whole multi-hour life.
        ///
        /// What remains true is that the holder is not *disadvantaged*: the
        /// launch policy nices the whole pane group (`apply_policy` above), so
        /// `verify.run` and the implementation agents all sit at nice 10 —
        /// parity, which is what AC-1 asks for. Starvation on Unix therefore
        /// comes from oversubscription, not from a priority gap, and it is
        /// reported rather than corrected: see `holder_activity` (AC-3) and
        /// the deferral text (AC-4). Lifting it belongs to `max_active` and to
        /// the gwt-side CPU burn tracked by #4397 / #4234.
        pub fn relieve_cap_for_lease_holders(
            &mut self,
            _holder_pids: &[u32],
        ) -> Result<bool, String> {
            Ok(false)
        }

        /// Signal every process in the group without waiting for reap.
        ///
        /// Idempotent: subsequent calls (including via `Drop`) become no-ops.
        /// Issue #3705: SIGKILL is sent immediately. A SIGTERM-then-sleep-then-
        /// SIGKILL sequence blocked the GUI event loop for 100ms per live PTY
        /// close and serialized `pane.*` behind it.
        pub fn terminate(&mut self) -> Result<(), String> {
            if self.termination_requested {
                return Ok(());
            }
            let pgid = self.pgid.ok_or("PTY process group was not captured")?;
            self.validate_root_identity()?;
            match killpg(pgid, Signal::SIGKILL) {
                Ok(()) => {}
                Err(Errno::ESRCH) => self.pgid = None,
                Err(error) => return Err(format!("killpg({pgid}, SIGKILL): {error}")),
            }
            self.termination_requested = true;
            Ok(())
        }

        /// Reclaim every group in the captured PTY session, even after its
        /// leader was reaped. Verification commands and shell job control can
        /// create other PGIDs without leaving that session.
        pub fn terminate_and_wait(&mut self, timeout: Duration) -> Result<bool, String> {
            if self.tree_exited {
                return Ok(true);
            }
            let session_id = self
                .session_id
                .ok_or("PTY process session was not captured")?
                .as_raw();
            let started = Instant::now();
            let mut captured = Vec::<NativeProcess>::new();
            loop {
                // A complete native census is required. An absent original
                // PGID or a partial/failed snapshot cannot prove tree exit.
                self.validate_root_identity()?;
                let (observed, census_stable) = session_processes(session_id)?;
                let mut live = !census_stable || observed.iter().any(|process| process.running);
                for process in observed {
                    if !captured.iter().any(|known| known.same_identity(&process)) {
                        captured.push(process);
                    }
                }
                for process in &captured {
                    let Some(current) = native_process(process.pid)? else {
                        continue;
                    };
                    if !process.same_identity(&current) || !current.running {
                        continue;
                    }
                    live = true;
                    self.validate_root_identity()?;
                    // SAFETY: the native PID/start identity was revalidated
                    // against a member of this exact captured PTY session.
                    if unsafe { libc::kill(current.pid, libc::SIGKILL) } == -1 {
                        let error = std::io::Error::last_os_error();
                        if error.raw_os_error() != Some(libc::ESRCH) {
                            return Err(format!(
                                "terminate PTY session member {}: {error}",
                                current.pid
                            ));
                        }
                    }
                }
                self.termination_requested = true;
                if !live {
                    self.pgid = None;
                    self.tree_exited = true;
                    return Ok(true);
                }
                // Re-enumerate after signalling or process-list churn: a
                // descendant may have forked and exited between enumeration
                // and its native read. An unstable zero census proves nothing.
                // Native zombies have exited and cannot keep working or holding a lease.
                let Some(remaining) = timeout.checked_sub(started.elapsed()) else {
                    return Ok(false);
                };
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
        }

        fn validate_root_identity(&self) -> Result<(), String> {
            let pid = self
                .session_id
                .ok_or("PTY process session was not captured")?
                .as_raw();
            let Some(current) = native_process(pid)? else {
                // The reaped leader need not remain alive while other groups
                // still belong to its captured session.
                return Ok(());
            };
            if current.session_id == pid
                && self
                    .captured_root
                    .is_some_and(|root| root.same_identity(&current))
            {
                Ok(())
            } else {
                Err(format!(
                    "PTY root identity changed for captured session {pid}"
                ))
            }
        }
    }

    #[derive(Clone, Copy)]
    struct NativeProcess {
        pid: libc::pid_t,
        session_id: libc::pid_t,
        started: [u64; 2],
        running: bool,
    }

    impl NativeProcess {
        fn same_identity(&self, other: &Self) -> bool {
            self.pid == other.pid && self.started == other.started
        }
    }

    fn process_session(pid: libc::pid_t) -> Result<Option<libc::pid_t>, String> {
        // SAFETY: getsid only observes the named PID.
        let session_id = unsafe { libc::getsid(pid) };
        if session_id >= 0 {
            return Ok(Some(session_id));
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            Ok(None)
        } else {
            Err(format!("observe session of process {pid}: {error}"))
        }
    }

    fn session_processes(session_id: libc::pid_t) -> Result<(Vec<NativeProcess>, bool), String> {
        let mut pids = native_process_ids()?;
        pids.sort_unstable();
        #[cfg(test)]
        if let Some(hook) = CENSUS_HOOK.with(|hook| hook.borrow_mut().take()) {
            hook();
        }
        if !pids.contains(&(std::process::id() as libc::pid_t)) {
            return Err("native process census omitted the observing process".into());
        }
        let mut processes = Vec::new();
        for &pid in &pids {
            if process_session(pid)? != Some(session_id) {
                continue;
            }
            if let Some(process) = native_process(pid)? {
                if process.session_id != session_id {
                    return Err(format!("process {pid} changed session during PTY census"));
                }
                processes.push(process);
            }
        }
        let mut after = native_process_ids()?;
        after.sort_unstable();
        Ok((processes, pids == after))
    }

    #[cfg(test)]
    thread_local! {
        pub(super) static CENSUS_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> = const {
            std::cell::RefCell::new(None)
        };
    }

    #[cfg(target_os = "linux")]
    fn native_process_ids() -> Result<Vec<libc::pid_t>, String> {
        std::fs::read_dir("/proc")
            .map_err(|error| format!("enumerate PTY session processes: {error}"))?
            .filter_map(|entry| match entry {
                Ok(entry) => entry
                    .file_name()
                    .to_str()
                    .and_then(|name| name.parse::<libc::pid_t>().ok())
                    .filter(|pid| *pid > 0)
                    .map(Ok),
                Err(error) => Some(Err(format!("incomplete process enumeration: {error}"))),
            })
            .collect()
    }

    #[cfg(target_os = "linux")]
    fn native_process(pid: libc::pid_t) -> Result<Option<NativeProcess>, String> {
        native_process_from_stat(pid, std::fs::read(format!("/proc/{pid}/stat")))
    }

    #[cfg(target_os = "linux")]
    fn native_process_from_stat(
        pid: libc::pid_t,
        stat: std::io::Result<Vec<u8>>,
    ) -> Result<Option<NativeProcess>, String> {
        let stat = match stat {
            Ok(stat) => stat,
            // A procfs file opened before exit can return ESRCH when read.
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.raw_os_error() == Some(libc::ESRCH) =>
            {
                return Ok(None);
            }
            Err(error) => return Err(format!("observe PTY session process {pid}: {error}")),
        };
        let fields = stat
            .iter()
            .rposition(|byte| *byte == b')')
            .and_then(|end| std::str::from_utf8(&stat[end + 1..]).ok())
            .map(|fields| fields.split_whitespace().collect::<Vec<_>>())
            .filter(|fields| fields.len() >= 20)
            .ok_or_else(|| format!("incomplete native process record for {pid}"))?;
        let session_id = fields[3]
            .parse()
            .map_err(|error| format!("invalid native session for {pid}: {error}"))?;
        let started = fields[19]
            .parse()
            .map_err(|error| format!("invalid native start identity for {pid}: {error}"))?;
        Ok(Some(NativeProcess {
            pid,
            session_id,
            started: [started, 0],
            running: !matches!(fields[0], "Z" | "X"),
        }))
    }

    #[cfg(all(test, target_os = "linux"))]
    #[test]
    fn native_process_stat_distinguishes_disappearance_from_observation_failure() {
        for code in [libc::ENOENT, libc::ESRCH] {
            let result =
                native_process_from_stat(4242, Err(std::io::Error::from_raw_os_error(code)));
            assert!(
                result.unwrap().is_none(),
                "process disappeared: errno {code}"
            );
        }
        for code in [libc::EACCES, libc::EPERM, libc::EIO] {
            let result =
                native_process_from_stat(4242, Err(std::io::Error::from_raw_os_error(code)));
            assert!(
                result.is_err(),
                "observation failure must remain an error: errno {code}"
            );
        }
    }

    #[cfg(target_os = "macos")]
    fn native_process_ids() -> Result<Vec<libc::pid_t>, String> {
        // SAFETY: the first call asks for capacity without writing a buffer.
        let count = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if count <= 0 {
            return Err(format!(
                "enumerate PTY session processes: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut pids = vec![0; count as usize + 32];
        // Retry a full buffer, never accept a silently truncated census.
        for _ in 0..4 {
            let size = i32::try_from(pids.len() * std::mem::size_of::<libc::pid_t>())
                .map_err(|_| "native process census exceeds buffer limit")?;
            // SAFETY: the initialized PID buffer has exactly `size` bytes.
            let count = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), size) };
            if count <= 0 {
                return Err(format!(
                    "observe native process census: {}",
                    std::io::Error::last_os_error()
                ));
            }
            if (count as usize) < pids.len() {
                pids.truncate(count as usize);
                pids.retain(|pid| *pid > 0);
                return Ok(pids);
            }
            pids.resize(pids.len() * 2, 0);
        }
        Err("native process census remained truncated".into())
    }

    #[cfg(target_os = "macos")]
    fn native_process(pid: libc::pid_t) -> Result<Option<NativeProcess>, String> {
        // SAFETY: proc_pidinfo receives the exact native structure and size.
        let info = unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of_val(&info) as libc::c_int;
            let observed = libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            );
            if observed != size {
                let error = std::io::Error::last_os_error();
                if observed == 0 && error.raw_os_error() == Some(libc::ESRCH) {
                    return Ok(None);
                }
                return Err(format!(
                    "incomplete native process record for {pid}: {error}"
                ));
            }
            if info.pbi_pid != pid as u32 {
                return Err(format!("native process identity changed for {pid}"));
            }
            info
        };
        let Some(session_id) = process_session(pid)? else {
            return Ok(None);
        };
        Ok(Some(NativeProcess {
            pid,
            session_id,
            started: [info.pbi_start_tvsec, info.pbi_start_tvusec],
            running: info.pbi_status != libc::SZOMB,
        }))
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn native_process_ids() -> Result<Vec<libc::pid_t>, String> {
        Err("native PTY session observation is unsupported on this Unix platform".into())
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    fn native_process(_pid: libc::pid_t) -> Result<Option<NativeProcess>, String> {
        Err("native PTY process observation is unsupported on this Unix platform".into())
    }

    impl Drop for ProcessGroup {
        fn drop(&mut self) {
            let _ = self.terminate();
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

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the regression owns a session leader and its independently grouped child"
    )]
    fn tree_confirmation_reclaims_a_separate_group_after_the_leader_is_reaped() {
        use std::{
            cell::Cell, io::Write, os::unix::process::CommandExt, process::Command, rc::Rc,
            time::Duration,
        };

        struct Fixture {
            leader: std::process::Child,
            descendant: Option<u32>,
            successor: Rc<Cell<Option<u32>>>,
        }

        impl Drop for Fixture {
            fn drop(&mut self) {
                for pid in [self.descendant, self.successor.get()]
                    .into_iter()
                    .flatten()
                {
                    // SAFETY: only reclaim the fixture child while it still
                    // belongs to the captured session, including on RED.
                    unsafe {
                        if libc::getsid(pid as libc::pid_t) == self.leader.id() as libc::pid_t {
                            libc::kill(pid as libc::pid_t, libc::SIGKILL);
                        }
                    }
                }
                let _ = self.leader.kill();
                let _ = self.leader.wait();
            }
        }

        let directory = tempfile::tempdir().unwrap();
        let child_path = directory.path().join("child.pid");
        let successor_path = child_path.with_extension("successor.pid");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "pty::process_group::tests::separate_group_child_fixture",
                "--nocapture",
            ])
            .env("GWT_SEPARATE_GROUP_CHILD_PID", &child_path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // SAFETY: setsid is async-signal-safe and gives this owned fixture
        // the same session/group topology as a portable_pty child.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut fixture = Fixture {
            leader: command.spawn().unwrap(),
            descendant: None,
            successor: Rc::new(Cell::new(None)),
        };
        let leader_pid = fixture.leader.id();
        let mut group = super::ProcessGroup::attach(leader_pid).unwrap();
        // test-hygiene: allow-wall-clock-deadline Bounds the real child's PID publication; ordering is established by the published PID, not elapsed time.
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        let child_pid = loop {
            if let Some(pid) = std::fs::read_to_string(&child_path)
                .ok()
                .and_then(|value| value.trim().parse::<u32>().ok())
            {
                break pid;
            }
            assert!(
                // test-hygiene: allow-wall-clock-deadline This watchdog prevents fixture startup from hanging the test.
                std::time::Instant::now() < deadline,
                "separate-group child did not publish its PID"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        fixture.descendant = Some(child_pid);
        // SAFETY: these calls only observe the exact fixture child.
        unsafe {
            assert_eq!(
                libc::getpgid(child_pid as libc::pid_t),
                child_pid as libc::pid_t
            );
            assert_eq!(
                libc::getsid(child_pid as libc::pid_t),
                leader_pid as libc::pid_t
            );
        }
        assert!(fixture_child_is_running(child_pid));

        // Match pane close's existing nonblocking kill followed by a later
        // finalizer: original-group absence must not hide the other group.
        let mut writer = fixture.leader.stdin.take().unwrap();
        group.terminate().unwrap();
        fixture.leader.wait().unwrap();

        struct ResetCensusHook;
        impl Drop for ResetCensusHook {
            fn drop(&mut self) {
                super::imp::CENSUS_HOOK.with(|hook| hook.borrow_mut().take());
            }
        }
        let _reset_hook = ResetCensusHook;
        let successor = Rc::clone(&fixture.successor);
        super::imp::CENSUS_HOOK.with(|hook| {
            *hook.borrow_mut() = Some(Box::new(move || {
                // The PID vector is already fixed. Let its only live owned
                // member fork a new child and exit before its native read.
                writer.write_all(b"go\n").unwrap();
                // test-hygiene: allow-wall-clock-deadline Bounds the external fork handshake, not a scheduling-order assertion.
                let deadline = std::time::Instant::now() + Duration::from_secs(20);
                let successor_pid = loop {
                    if let Some(pid) = std::fs::read_to_string(&successor_path)
                        .ok()
                        .and_then(|value| value.trim().parse::<u32>().ok())
                    {
                        successor.set(Some(pid));
                        break pid;
                    }
                    assert!(
                        // test-hygiene: allow-wall-clock-deadline This watchdog prevents the fork handshake from hanging the test.
                        std::time::Instant::now() < deadline,
                        "successor did not publish its PID"
                    );
                    std::thread::sleep(Duration::from_millis(100));
                };
                assert!(fixture_child_is_running(successor_pid));
                while fixture_child_is_running(child_pid) {
                    assert!(
                        // test-hygiene: allow-wall-clock-deadline Observe the old process exit before allowing the native census to continue.
                        std::time::Instant::now() < deadline,
                        "original child did not exit after publishing its successor"
                    );
                    std::thread::sleep(Duration::from_millis(100));
                }
            }));
        });
        // The test hook's 20s startup watchdog is charged to this operation;
        // retain another 2s for the normal confirmation after the handshake.
        let confirmed = group.terminate_and_wait(Duration::from_secs(22)).unwrap();
        let survived_confirmation = fixture_child_is_running(child_pid);
        let successor_pid = fixture.successor.get().expect("the census hook must run");
        let successor_survived_confirmation = fixture_child_is_running(successor_pid);
        assert!(confirmed, "the captured tree did not finish teardown");
        assert!(
            !survived_confirmation,
            "tree confirmation succeeded while separate-group child {child_pid} was running"
        );
        assert!(
            !successor_survived_confirmation,
            "tree confirmation succeeded while successor {successor_pid} omitted from the first census was running"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "the subprocess fixture intentionally creates an independent owned process group"
    )]
    fn separate_group_child_fixture() {
        use std::{os::unix::process::CommandExt, process::Command};

        let Some(path) = std::env::var_os("GWT_SEPARATE_GROUP_CHILD_PID") else {
            return;
        };
        // SAFETY: this is a dedicated subprocess fixture. Ignoring HUP keeps
        // leader exit from independently terminating the observed child.
        unsafe {
            libc::signal(libc::SIGHUP, libc::SIG_IGN);
        }
        let successor_path = std::path::Path::new(&path).with_extension("successor.pid");
        let mut child = Command::new("/bin/sh")
            .args([
                "-c",
                "trap '' HUP; IFS= read -r go || exit 0; [ \"$go\" = go ] || exit 0; /bin/sleep 600 & printf '%s\\n' \"$!\" > \"$GWT_SEPARATE_GROUP_SUCCESSOR_PID\"",
            ])
            .env("GWT_SEPARATE_GROUP_SUCCESSOR_PID", successor_path)
            .process_group(0)
            .spawn()
            .unwrap();
        std::fs::write(path, child.id().to_string()).unwrap();
        child.wait().unwrap();
    }

    #[cfg(target_os = "linux")]
    fn fixture_child_is_running(pid: u32) -> bool {
        match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
            Ok(stat) => {
                let (_, fields) = stat.rsplit_once(')').expect("native process stat");
                !matches!(fields.split_whitespace().next(), Some("Z" | "X"))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => panic!("could not observe fixture child {pid}: {error}"),
        }
    }

    #[cfg(target_os = "macos")]
    fn fixture_child_is_running(pid: u32) -> bool {
        // SAFETY: proc_pidinfo fills this correctly sized native structure.
        unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of_val(&info) as libc::c_int;
            let observed = libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDTBSDINFO,
                0,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            );
            if observed == 0 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                return false;
            }
            assert_eq!(observed, size, "could not observe fixture child {pid}");
            info.pbi_status != libc::SZOMB
        }
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
        let mut ancestor = super::imp::ProcessGroup::attach(pid).expect("attach ancestor Job");
        ancestor
            .apply_policy(
                pid,
                ProcessPolicy {
                    priority: ProcessPriority::BelowNormal,
                    cpu_limit_percent: Some(40),
                },
            )
            .unwrap();
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
        assert!(!group.relieve_cap_for_lease_holders(&[outsider]).unwrap());
        assert_eq!(group.cpu_cap_percent(), Some(25), "a holder elsewhere");
        // A holder that already exited is simply not here: no error, no change.
        assert!(!group
            .relieve_cap_for_lease_holders(&[u32::MAX - 3])
            .unwrap());
        assert_eq!(group.cpu_cap_percent(), Some(25), "a vanished holder");

        assert!(group
            .relieve_cap_for_lease_holders(&[outsider, pid])
            .unwrap());
        assert!(ancestor
            .relieve_cap_for_lease_holders(&[outsider, pid])
            .unwrap());
        assert_eq!(group.cpu_cap_percent(), None, "the holder is here");
        assert_eq!(
            ancestor.cpu_cap_percent(),
            None,
            "the ancestor is also relieved"
        );
        assert!(!group.relieve_cap_for_lease_holders(&[pid]).unwrap());
        assert_eq!(
            gwt_core::process_tree::process_priority_class(pid).unwrap(),
            gwt_core::process_tree::ProcessPriorityClass::BelowNormal,
            "relief preserves the scheduling priority"
        );

        assert!(group.relieve_cap_for_lease_holders(&[]).unwrap());
        assert!(ancestor.relieve_cap_for_lease_holders(&[]).unwrap());
        assert_eq!(group.cpu_cap_percent(), Some(25), "the lease is free");
        assert_eq!(
            ancestor.cpu_cap_percent(),
            Some(40),
            "ancestor policy is restored"
        );

        group.terminate().expect("request Job termination");
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
