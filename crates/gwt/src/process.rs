//! Cross-platform process liveness probe shared by the daemon
//! bootstrap callers.
//!
//! This module centralises the `kill(pid, 0)` probe that several
//! daemon-related modules (`cli::daemon::mod`, `daemon_publisher`,
//! `main`) used to duplicate. Three identical 10-line helpers had
//! drifted slightly (`is_process_alive_pid`, `is_alive`,
//! `is_subscriber_pid_alive`); consolidating into one definition
//! removes that drift surface and makes the platform-conditional
//! behaviour explicit in a single place.
//!
//! Every daemon-bootstrap caller now shares this one predicate. The GUI front
//! door used to run a narrower `|pid| pid == std::process::id()` variant that
//! classified a live daemon as dead; Issue #2338 resolved that by removing the
//! front door's endpoint-slot handling entirely rather than by giving it a
//! second liveness definition.

fn resolve_username(whoami_value: Option<&str>, env_value: Option<&str>) -> String {
    whoami_value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| env_value.map(str::trim).filter(|value| !value.is_empty()))
        .unwrap_or("unknown")
        .to_string()
}

fn normalize_hostname(value: Option<&str>) -> String {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
        .map(|value| value.trim_end_matches(".local").to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "unknown-host".to_string())
}

/// Stable owner identity for remote Issue claims. The host component prevents
/// same-user/PID collisions across machines; old persisted claims remain
/// readable because parsing still treats owner as an opaque string.
pub fn current_claim_owner() -> String {
    let hostname = current_hostname();
    format!("{}:{}:{}", hostname, current_username(), std::process::id())
}

pub fn current_hostname() -> String {
    normalize_hostname(whoami::hostname().ok().as_deref())
}

/// Issue #4852 AC-2: the `hostname:user` prefix of a claim owner minted by
/// [`current_claim_owner`], without the pid. `None` when the owner does not
/// carry the three-part shape (a bare hostname from `queue push`, or an
/// opaque legacy owner), so a prefix is never invented for it.
pub fn claim_owner_host_user(owner: &str) -> Option<&str> {
    let (prefix, pid) = owner.trim().rsplit_once(':')?;
    if pid.is_empty() || !pid.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    prefix.contains(':').then_some(prefix)
}

/// Issue #4852 AC-2: whether two claim owners name the same `hostname:user`,
/// whatever pid each carries. A Monitor process that restarted keeps meeting
/// the claim comments its predecessor pid wrote on the same host; those are
/// its own claims, not a foreign hold, so pid is deliberately not compared.
/// Owners without the `hostname:user:pid` shape only match exactly.
pub fn same_host_and_user(owner_a: &str, owner_b: &str) -> bool {
    if owner_a.trim() == owner_b.trim() {
        return true;
    }
    match (
        claim_owner_host_user(owner_a),
        claim_owner_host_user(owner_b),
    ) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

/// Return the current username, falling back to the platform environment.
pub fn current_username() -> String {
    let whoami_value = whoami::username().ok();
    let env_var = if cfg!(target_os = "windows") {
        "USERNAME"
    } else {
        "USER"
    };
    let env_value = std::env::var(env_var).ok();
    resolve_username(whoami_value.as_deref(), env_value.as_deref())
}

/// Return whether `pid` is alive, using a signal-zero probe on Unix and
/// a direct process-handle probe on Windows. Permission failures do not
/// establish that an owner is dead.
pub fn is_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: kill(pid, 0) returns 0 if the process exists, -1
        // with ESRCH if it does not. We never deliver a real signal.
        let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
        if rc == 0 {
            return true;
        }
        let err = std::io::Error::last_os_error();
        // EPERM means the process exists but we lack permission to
        // signal it — still alive from the bootstrap caller's POV.
        matches!(err.raw_os_error(), Some(libc::EPERM))
    }
    #[cfg(not(unix))]
    {
        // Issue #3526: the Windows daemon (named-pipe transport) persists
        // real endpoint / authority-fence PIDs, so bootstrap resolution
        // needs a truthful liveness answer here. A permanent `false` would
        // make every consumer delete a live daemon's endpoint file on each
        // bootstrap call and let the GUI tick double-drive the scan.
        is_host_process_alive(pid)
    }
}

/// Probe only the requested PID. Windows daemon event publishing calls this
/// for every output chunk; a full process snapshot here consumes a core even
/// when `sysinfo` is asked to refresh just one PID (Issue #4799).
pub fn is_host_process_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    #[cfg(unix)]
    {
        is_process_alive(pid)
    }
    #[cfg(not(unix))]
    {
        use windows::{
            core::HRESULT,
            Win32::{
                Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, WAIT_OBJECT_0},
                System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE},
            },
        };

        // SAFETY: request only wait access to this PID, without inheritance.
        let handle = match unsafe { OpenProcess(PROCESS_SYNCHRONIZE, false, pid) } {
            Ok(handle) => handle,
            // Access denied or another uncertain failure must not authorize
            // replacing a live owner. An invalid PID is definitively absent.
            Err(error) => return error.code() != HRESULT::from_win32(ERROR_INVALID_PARAMETER.0),
        };
        // SAFETY: the owned process handle stays valid through the zero-timeout
        // wait and is then closed exactly once. A signaled process has exited,
        // including one whose actual exit code happens to be STILL_ACTIVE.
        unsafe {
            let alive = WaitForSingleObject(handle, 0) != WAIT_OBJECT_0;
            let _ = CloseHandle(handle);
            alive
        }
    }
}

/// Issue #3906 AC-8: whether `pid` runs somewhere under `ancestor` in the
/// process tree (an agent pane's `gwtd verify.run`, for example). Walks the
/// parent chain through `sysinfo`; a missing process or a chain longer than
/// 64 hops counts as "not ours" so the drain never blocks on a stranger.
pub fn is_descendant_of(pid: u32, ancestor: u32) -> bool {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    if pid == 0 || ancestor == 0 || pid == ancestor {
        return false;
    }
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let mut current = sysinfo::Pid::from_u32(pid);
    for _ in 0..64 {
        let Some(parent) = system.process(current).and_then(sysinfo::Process::parent) else {
            return false;
        };
        if parent.as_u32() == ancestor {
            return true;
        }
        if parent.as_u32() <= 1 {
            return false;
        }
        current = parent;
    }
    false
}

/// Return the start identity of one host process incarnation.
///
/// A PID by itself is not a durable process identity because operating
/// systems recycle it. Cross-process launch fences persist this value beside
/// the PID and compare both before treating a previous Host as still live.
///
/// The value is an opaque token compared only for equality. On Linux it is
/// the `starttime` tick count from `/proc/<pid>/stat`: the wall-clock start
/// time is `btime + ticks / CLK_TCK`, and WSL moves `btime` while the process
/// lives, so a live Host would otherwise read as a different incarnation
/// (Issue #5089). Elsewhere the OS start time is already fixed per process.
/// Use [`host_process_start_epoch_secs`] when wall-clock seconds are needed.
pub fn host_process_start_time(pid: u32) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        proc_stat_start_identity(&stat)
    }
    #[cfg(not(target_os = "linux"))]
    {
        host_process_start_epoch_secs(pid)
    }
}

/// Start identity of a process already present in a `sysinfo` snapshot: the
/// same token as [`host_process_start_time`], or `0` when it is unreadable.
pub fn snapshot_process_start_identity(pid: u32, process: &sysinfo::Process) -> u64 {
    #[cfg(target_os = "linux")]
    {
        let _ = process;
        host_process_start_time(pid).unwrap_or(0)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        process.start_time()
    }
}

/// Parse the `starttime` field (22) of a `/proc/<pid>/stat` line. `comm` may
/// contain spaces and parentheses, so fields are counted after the last `)`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn proc_stat_start_identity(stat: &str) -> Option<u64> {
    let (_, fields) = stat.rsplit_once(')')?;
    fields
        .split_whitespace()
        .nth(19)?
        .parse::<u64>()
        .ok()
        .filter(|ticks| *ticks > 0)
}

/// Return the wall-clock start time of one host process in Unix seconds.
///
/// Only ordering checks against wall-clock stamps should use this; it is not
/// stable across clock steps on Linux and must not be compared for identity.
pub fn host_process_start_epoch_secs(pid: u32) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

    let mut system = System::new();
    let pid = sysinfo::Pid::from_u32(pid);
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    system
        .process(pid)
        .map(sysinfo::Process::start_time)
        .filter(|started_at| *started_at > 0)
}

/// Return whether a Unix process group still contains any process.
///
/// PTY children are session/process-group leaders. The direct leader can exit
/// while a foreground descendant remains the execution writer, so PID
/// liveness alone is not sufficient recovery evidence. Windows process trees
/// are owned by the kill-on-close Job Object and use direct child identity.
pub fn is_process_group_alive(process_group_id: u32) -> bool {
    if process_group_id == 0 || process_group_id > i32::MAX as u32 {
        return false;
    }
    #[cfg(unix)]
    {
        // SAFETY: signal 0 performs only an existence/permission probe. A
        // negative pid addresses the whole process group.
        let rc = unsafe { libc::kill(-(process_group_id as libc::pid_t), 0) };
        if rc == 0 {
            return true;
        }
        matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EPERM)
        )
    }
    #[cfg(not(unix))]
    {
        false
    }
}

/// Check the exact PTY child or any surviving member of its process group.
pub fn exact_pty_process_tree_is_alive(child_pid: u32, child_started_at: u64) -> bool {
    host_process_start_time(child_pid) == Some(child_started_at)
        || is_process_group_alive(child_pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn username_resolution_prefers_whoami_then_env_then_unknown() {
        assert_eq!(
            resolve_username(Some("  whoami-user  "), Some("  env-user  ")),
            "whoami-user"
        );
        assert_eq!(
            resolve_username(Some("  "), Some("  env-user  ")),
            "env-user"
        );
        assert_eq!(resolve_username(None, Some(" runner ")), "runner");
        assert_eq!(resolve_username(None, None), "unknown");
    }

    #[test]
    fn hostname_normalization_is_case_insensitive_and_removes_local_suffix() {
        assert_eq!(normalize_hostname(Some(" MacBook.LOCAL ")), "macbook");
        assert_eq!(normalize_hostname(Some("  ")), "unknown-host");
        assert_eq!(normalize_hostname(None), "unknown-host");
    }

    #[test]
    fn claim_owner_contains_host_username_and_pid() {
        let owner = current_claim_owner();
        assert_eq!(owner.split(':').count(), 3);
        assert!(owner.ends_with(&format!(":{}", std::process::id())));
    }

    /// Issue #4852 AC-2: a claim written by an earlier pid of this host/user
    /// is our own; another host or user with the same pid is not.
    #[test]
    fn same_host_and_user_ignores_pid_but_not_host_or_user() {
        let mine = current_claim_owner();
        let other_pid = format!(
            "{}:{}:{}",
            current_hostname(),
            current_username(),
            std::process::id().wrapping_add(1)
        );
        assert!(same_host_and_user(&mine, &mine));
        assert!(same_host_and_user(&mine, &other_pid));
        assert!(same_host_and_user(&other_pid, &mine));
        assert!(same_host_and_user("host-a:alice:10", "host-a:alice:20"));
        assert!(!same_host_and_user("host-a:alice:10", "host-b:alice:10"));
        assert!(!same_host_and_user("host-a:alice:10", "host-a:bob:10"));
        // Owners without the three-part shape never gain a prefix match.
        assert!(!same_host_and_user("host-a", "host-a:alice:10"));
        assert!(!same_host_and_user("host-a/session", "host-a/session-2"));
        assert!(same_host_and_user("host-a/session", "host-a/session"));
        assert_eq!(
            claim_owner_host_user("host-a:alice:10"),
            Some("host-a:alice")
        );
        assert_eq!(claim_owner_host_user("host-a:alice:x"), None);
        assert_eq!(claim_owner_host_user("alice:10"), None);
    }

    #[test]
    fn pid_zero_is_never_alive() {
        assert!(!is_process_alive(0));
        assert!(!is_host_process_alive(0));
    }

    #[test]
    fn current_host_process_is_alive_on_every_supported_platform() {
        assert!(is_host_process_alive(std::process::id()));
    }

    #[cfg(unix)]
    #[test]
    fn current_unix_process_group_is_detected() {
        // SAFETY: getpgrp has no preconditions or side effects.
        let process_group = unsafe { libc::getpgrp() };
        assert!(process_group > 0);
        assert!(is_process_group_alive(process_group as u32));
        assert!(!is_process_group_alive(0));
    }

    #[test]
    fn host_process_start_time_distinguishes_the_current_process_from_missing_pid() {
        assert!(host_process_start_time(std::process::id()).is_some_and(|value| value > 0));
        assert_eq!(host_process_start_time(0), None);
        assert_eq!(host_process_start_time(i32::MAX as u32), None);
    }

    /// `/proc/<pid>/stat` line for one incarnation; `comm` may contain spaces
    /// and parentheses, so parsing must anchor on the last `)`.
    fn proc_stat_line(pid: u32, start_ticks: u64) -> String {
        format!(
            "{pid} (gwt (host) x) S 1 {pid} {pid} 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 4 0 \
             {start_ticks} 123456789 2048 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 17 3 0 0"
        )
    }

    #[test]
    fn process_start_identity_does_not_move_when_the_boot_time_moves() {
        // Issue #5089: WSL reported btime 1791187159 -> 160 -> 161 for one
        // live PID whose start_ticks stayed 10546425. An identity derived
        // from `btime + start_ticks / CLK_TCK` changes with each reading;
        // the identity must come from the per-process tick count alone.
        let stat = proc_stat_line(1_471_923, 10_546_425);
        let wall_clock_identities: std::collections::BTreeSet<u64> =
            [1_791_187_159u64, 1_791_187_160, 1_791_187_161]
                .into_iter()
                .map(|btime| btime + 10_546_425 / 100)
                .collect();
        assert_eq!(wall_clock_identities.len(), 3);

        // The stat line carries no boot time, so the identity is the tick
        // count for every one of those readings.
        assert_eq!(proc_stat_start_identity(&stat), Some(10_546_425));
    }

    #[test]
    fn process_start_identity_rejects_a_reused_pid_and_malformed_stat() {
        let original = proc_stat_start_identity(&proc_stat_line(4_242, 10_546_425));
        let reused = proc_stat_start_identity(&proc_stat_line(4_242, 10_546_426));
        assert!(original.is_some());
        assert_ne!(original, reused);
        assert_eq!(proc_stat_start_identity(""), None);
        assert_eq!(proc_stat_start_identity("4242 (gwt) S 1 2"), None);
        assert_eq!(proc_stat_start_identity(&proc_stat_line(4_242, 0)), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_host_process_start_time_is_the_proc_start_tick_count() {
        let pid = std::process::id();
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        assert_eq!(
            host_process_start_time(pid),
            proc_stat_start_identity(&stat)
        );
    }

    #[test]
    fn host_process_start_epoch_secs_is_wall_clock_seconds() {
        let started = host_process_start_epoch_secs(std::process::id()).unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(started <= now + 5 && started > 1_600_000_000);
        assert_eq!(host_process_start_epoch_secs(0), None);
    }

    #[test]
    fn current_process_is_alive() {
        assert!(is_process_alive(std::process::id()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_liveness_rejects_missing_and_exited_processes() {
        assert!(!is_host_process_alive(u32::MAX));
        let request =
            gwt_core::process::ProcessPlanRequest::new("cmd").args(["/D", "/C", "exit 259"]);
        let mut child = gwt_core::process::resolved_command(request)
            .expect("resolve cmd")
            .spawn()
            .expect("spawn child");
        assert_eq!(child.wait().expect("wait for child").code(), Some(259));
        // Keep its handle alive: the terminated process object still exists,
        // and its exit code equals STILL_ACTIVE, but it is no longer running.
        assert!(!is_host_process_alive(child.id()));
    }

    #[cfg(windows)]
    #[test]
    fn windows_exact_process_identity_rejects_a_reused_pid() {
        let pid = std::process::id();
        let started_at = host_process_start_time(pid).expect("current process start time");
        assert!(exact_pty_process_tree_is_alive(pid, started_at));
        assert!(!exact_pty_process_tree_is_alive(pid, started_at - 1));
    }

    #[cfg(windows)]
    #[test]
    fn windows_liveness_probe_stays_within_cpu_budget() {
        use windows::Win32::{
            Foundation::FILETIME,
            System::Threading::{GetCurrentThread, GetThreadTimes},
        };

        fn thread_cpu() -> std::time::Duration {
            let (mut created, mut exited, mut kernel, mut user) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            // SAFETY: all output pointers are valid; the pseudo handle denotes
            // this calling thread and must not be closed.
            unsafe {
                GetThreadTimes(
                    GetCurrentThread(),
                    &mut created,
                    &mut exited,
                    &mut kernel,
                    &mut user,
                )
                .expect("read thread CPU time");
            }
            let ticks = |time: FILETIME| {
                (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime)
            };
            // test-hygiene: allow-short-duration converts measured CPU ticks; no wall-clock wait
            std::time::Duration::from_micros((ticks(kernel) + ticks(user)) / 10)
        }

        let before = thread_cpu();
        for _ in 0..1000 {
            assert!(is_host_process_alive(std::process::id()));
        }
        let cpu = thread_cpu() - before;
        eprintln!("1000 Windows PID probes consumed {cpu:?} of thread CPU");
        assert!(
            cpu < std::time::Duration::from_millis(500),
            "PID probe CPU budget exceeded: {cpu:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn far_unused_pid_is_not_alive() {
        // Use `i32::MAX as u32` so the value stays positive after the
        // `pid as libc::pid_t` cast inside `is_process_alive`. Going
        // higher (e.g. `u32::MAX - 1`) wraps to a negative `pid_t` and
        // `kill(-N, 0)` probes process *group* `N` instead of a far
        // PID, which is a different semantic and can flake on
        // runners where group 2 exists.
        //
        // `i32::MAX` (~2.1 billion) is far past any realistic OS
        // pid_t allocation window today; if this ever flakes on a CI
        // runner we'll have learned that pid recycling has reached
        // extreme territory.
        assert!(!is_process_alive(i32::MAX as u32));
    }
}
