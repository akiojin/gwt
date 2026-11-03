//! Explicit, generation-bound fleet disposition; inventory is never authority.

use std::{collections::HashSet, path::Path};

use gwt_github::{ApiError, SpecOpsError};
use serde::Deserialize;
use serde_json::{json, Value};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use super::{CliEnv, CliParseError};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FleetTarget {
    pub id: String,
    pub session_id: Option<String>,
    pub child_pid: Option<u32>,
    pub child_started_at: Option<u64>,
    pub pid: Option<u32>,
    pub started_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetCommand {
    Inventory,
    Orphans,
    Disposition {
        targets: Vec<FleetTarget>,
        allow_unknown: bool,
        orphan_only: bool,
    },
}

pub(super) fn parse_targets(value: &Value) -> Result<Vec<FleetTarget>, CliParseError> {
    let targets: Vec<FleetTarget> = serde_json::from_value(value.clone())
        .map_err(|error| CliParseError::InvalidJson(format!("invalid fleet targets: {error}")))?;
    if targets.is_empty() || targets.iter().any(|target| target.id.trim().is_empty()) {
        return Err(CliParseError::InvalidJson(
            "fleet targets must be a nonempty explicit set".into(),
        ));
    }
    let mut ids = HashSet::new();
    if targets.iter().any(|target| !ids.insert(&target.id)) {
        return Err(CliParseError::InvalidJson(
            "fleet targets contain duplicate ids".into(),
        ));
    }
    Ok(targets)
}

fn preflight(rows: &[Value], targets: &[FleetTarget], allow_unknown: bool) -> Result<(), String> {
    if !allow_unknown
        && targets.iter().any(|target| {
            rows.iter()
                .find(|row| row["id"] == target.id)
                .is_some_and(|row| row["delivery_state"] == "unknown")
        })
    {
        return Err("fleet disposition refused: target set contains unknown delivery; explicitly set allow_unknown to override".into());
    }
    Ok(())
}

fn validate_target(row: &Value, target: &FleetTarget) -> Result<(), String> {
    if row["claim_holder"] == true
        || row["execution_holder"] == true
        || row["delivery_evidence"]["entry_is_execution_holder"] == true
    {
        return Err("current claim holder is protected; use its owner lifecycle operation".into());
    }
    if row["session_id"].as_str() != target.session_id.as_deref() {
        return Err("target Session changed since selection".into());
    }
    if row["pane"].is_object() {
        if target.session_id.as_deref().is_none_or(str::is_empty) {
            return Err("pane Session identity is unavailable".into());
        }
        if row["pane"]["child_pid"].as_u64() != target.child_pid.map(u64::from)
            || row["pane"]["child_started_at"].as_u64() != target.child_started_at
            || target.child_pid.is_some() != target.child_started_at.is_some()
        {
            return Err("target runtime generation changed since selection".into());
        }
    } else if row["process"].is_object()
        && (row["process"]["pid"].as_u64() != target.pid.map(u64::from)
            || row["process"]["started_at"].as_u64() != target.started_at
            || target.started_at.is_none())
    {
        return Err("target process generation changed or is unavailable".into());
    }
    Ok(())
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: FleetCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    if matches!(command, FleetCommand::Inventory) {
        return super::fleet::run(env, out);
    }
    let report = super::fleet::collect(env.repo_path());
    if matches!(command, FleetCommand::Orphans) {
        let entries: Vec<_> = report
            .entries
            .iter()
            .filter(|entry| entry.process.is_some() && entry.pane.is_none())
            .collect();
        out.push_str(&format!(
            "{}\n",
            json!({"sources":report.sources,"orphans":entries})
        ));
        return Ok(0);
    }
    if !super::pm::caller_is_registered_pm(env.repo_path()) {
        return Err(config_error(
            "fleet disposition requires the registered project PM",
        ));
    }
    let FleetCommand::Disposition {
        targets,
        allow_unknown,
        orphan_only,
    } = command
    else {
        unreachable!()
    };
    let rows: Vec<Value> = report
        .entries
        .iter()
        .map(|entry| serde_json::to_value(entry).expect("fleet entry is serializable"))
        .collect();
    preflight(&rows, &targets, allow_unknown).map_err(config_error)?;
    let panes_observed = report
        .sources
        .iter()
        .any(|source| source.name == "panes" && source.status == "ok");
    let receipts: Vec<_> = targets.iter().map(|target| {
        let result = (|| {
            let row = rows.iter().find(|row| row["id"] == target.id).ok_or("target no longer observed")?;
            validate_target(row,target)?;
            if orphan_only && !row["pane"].is_null() { return Err("target currently belongs to a pane".into()); }
            let duplicate_non_holder = row["claim_holder"] == false && row["duplicate_count"].as_u64().is_some_and(|count|count>1);
            if !duplicate_non_holder && !matches!(row["delivery_state"].as_str(),Some("delivered"|"recovery_connected"))
                && !(allow_unknown && row["delivery_state"] == "unknown") {
                return Err("undelivered work is protected".to_string());
            }
            if row["pane"].is_object() {
                let pane_id = row["pane"]["id"].as_str().or_else(|| row["pane"]["window_id"].as_str()).ok_or("pane identity unavailable")?;
                super::pane::fleet_close(pane_id,target.session_id.as_deref().unwrap_or_default(),target.child_pid,target.child_started_at)
            } else if row["process"].is_object() {
                if !panes_observed { return Err("orphan termination requires a complete current pane observation".into()); }
                terminate_orphan(row, target)
            } else {
                Err("Monitor-only rows have no process or pane to close; use issue.monitor lifecycle operations".into())
            }
        })();
        match result {
            Ok(detail) => json!({"id":target.id,"ok":true,"detail":detail}),
            Err(reason) => json!({"id":target.id,"ok":false,"reason":reason}),
        }
    }).collect();
    out.push_str(&format!("{}\n", json!({"receipts":receipts})));
    Ok(0)
}

/// Keep the OS process object fixed from generation validation through signalling.
struct BoundOrphanProcess {
    #[cfg(target_os = "linux")]
    handle: std::os::fd::OwnedFd,
    #[cfg(windows)]
    handle: windows::core::Owned<windows::Win32::Foundation::HANDLE>,
}

impl BoundOrphanProcess {
    fn capture(pid: u32, expected_started_at: u64) -> Result<Self, String> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::FromRawFd;

            let os_pid = libc::pid_t::try_from(pid).map_err(|_| "invalid orphan pid")?;
            // SAFETY: pidfd_open takes a positive PID and zero flags, and returns
            // a new owned descriptor. Failure must never fall back to kill(pid).
            let descriptor = unsafe { libc::syscall(libc::SYS_pidfd_open, os_pid, 0_u32) };
            if descriptor < 0 {
                return Err(format!(
                    "cannot acquire a stable process handle: pidfd_open: {}",
                    std::io::Error::last_os_error()
                ));
            }
            // SAFETY: the successful syscall returned a newly owned descriptor.
            let handle = unsafe { std::os::fd::OwnedFd::from_raw_fd(descriptor as i32) };
            // Open first: if this incarnation exits after validation, the
            // descriptor cannot be redirected to a process reusing its PID.
            if crate::process::host_process_start_time(pid) != Some(expected_started_at) {
                return Err("orphan process generation changed".into());
            }
            Ok(Self { handle })
        }
        #[cfg(windows)]
        {
            use windows::{
                core::Owned,
                Win32::{
                    Foundation::FILETIME,
                    System::Threading::{
                        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                        PROCESS_TERMINATE,
                    },
                },
            };

            // SAFETY: OpenProcess returns an owned, non-inherited handle with
            // precisely the rights needed for validation and termination.
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
                    false,
                    pid,
                )
                .map(|handle| Owned::new(handle))
            }
            .map_err(|error| format!("cannot acquire a stable process handle: {error}"))?;
            let mut creation = FILETIME::default();
            let mut exit = FILETIME::default();
            let mut kernel = FILETIME::default();
            let mut user = FILETIME::default();
            // SAFETY: this handle remains owned throughout the query, and all
            // four FILETIME outputs are valid writable values.
            unsafe { GetProcessTimes(*handle, &mut creation, &mut exit, &mut kernel, &mut user) }
                .map_err(|error| format!("cannot verify orphan process generation: {error}"))?;
            let ticks =
                (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime);
            // Match the existing Windows host identity (Unix epoch seconds).
            let started_at = (ticks / 10_000_000).checked_sub(11_644_473_600);
            if started_at != Some(expected_started_at) {
                return Err("orphan process generation changed".into());
            }
            Ok(Self { handle })
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = (pid, expected_started_at);
            // macOS needs a separately validated audit-token signalling path;
            // a PID-only kill, even after a start-time check, is unsafe.
            Err("orphan termination refused: a stable process handle is unavailable on this platform"
                .into())
        }
    }

    fn terminate(&self) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::AsRawFd;

            // SAFETY: the descriptor stays owned for this call; a null info
            // pointer and zero flags request SIGKILL for this fixed process.
            let result = unsafe {
                libc::syscall(
                    libc::SYS_pidfd_send_signal,
                    self.handle.as_raw_fd(),
                    libc::SIGKILL,
                    std::ptr::null::<libc::siginfo_t>(),
                    0_u32,
                )
            };
            if result < 0 {
                return Err(format!(
                    "pidfd_send_signal failed: {}",
                    std::io::Error::last_os_error()
                ));
            }
            Ok(())
        }
        #[cfg(windows)]
        {
            // SAFETY: this is the same owned process handle whose creation
            // time was checked, opened with PROCESS_TERMINATE access.
            unsafe { windows::Win32::System::Threading::TerminateProcess(*self.handle, 1) }
                .map_err(|error| format!("TerminateProcess failed: {error}"))
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            Err("orphan termination refused: a stable process handle is unavailable on this platform"
                .into())
        }
    }
}

fn terminate_orphan(row: &Value, target: &FleetTarget) -> Result<Value, String> {
    let worktree = row["worktree_path"]
        .as_str()
        .filter(|path| !path.is_empty())
        .ok_or("orphan worktree identity unavailable")?;
    let process_worktree = row["process"]["worktree_path"]
        .as_str()
        .ok_or("orphan process source identity unavailable")?;
    let cwd = row["process"]["cwd"]
        .as_str()
        .ok_or("orphan cwd unavailable")?;
    let worktree = dunce::canonicalize(worktree).map_err(|error| error.to_string())?;
    if dunce::canonicalize(process_worktree).map_err(|error| error.to_string())? != worktree
        || !dunce::canonicalize(cwd)
            .map_err(|error| error.to_string())?
            .starts_with(&worktree)
        || super::trusted_store::trusted_dir_for_worktree(&worktree).is_none()
        || row["process"]["pid"].as_u64() != target.pid.map(u64::from)
        || row["process"]["started_at"].as_u64() != target.started_at
    {
        return Err("orphan worktree or process source identity changed".into());
    }
    let owner = row["owner_number"]
        .as_u64()
        .filter(|number| *number > 0)
        .ok_or(
            "orphan owner identity unavailable; current claim association cannot be disproved",
        )?;
    let sessions_dir = gwt_core::paths::gwt_sessions_dir();
    let prefs_path = crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&worktree);
    super::execution_state::with_fleet_cleanup_lease(
        &worktree,
        &sessions_dir,
        target.session_id.as_deref(),
        || {
            if let Some(parent) = prefs_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let _monitor_lease = gwt_core::operation_deadline::NamedFileLock::try_acquire(
                &prefs_path.with_extension("lock"),
                "fleet orphan termination",
            )?;
            let prefs = crate::issue_monitor::load_issue_monitor_prefs(&prefs_path)?;
            // An orphan has no pane identity to prove it is a non-holder.
            // Delivery overrides never override uncertainty about authority.
            if prefs.launched_claims.contains_key(&owner)
                || prefs
                    .launched_issues
                    .iter()
                    .any(|entry| entry.issue_number == owner)
                || prefs
                    .launching_issues
                    .iter()
                    .any(|entry| entry.issue_number == owner)
            {
                return Err(std::io::Error::other(
                    "current claim holder is protected; orphan claim association is unavailable",
                ));
            }
            terminate_orphan_process_tree(row, target).map_err(std::io::Error::other)
        },
    )
    .map_err(|error| error.to_string())
}

fn terminate_orphan_process_tree(row: &Value, target: &FleetTarget) -> Result<Value, String> {
    let pid = target.pid.ok_or("orphan target requires pid")?;
    let start = target
        .started_at
        .ok_or("orphan target requires started_at")?;
    if pid == std::process::id()
        || crate::process::is_descendant_of(std::process::id(), pid)
            .map_err(|error| format!("PM process ancestry unavailable: {error}"))?
    {
        return Err("the calling PM process tree is protected".into());
    }
    let expected_cwd = row["process"]["cwd"]
        .as_str()
        .ok_or("orphan cwd unavailable")?;
    let root = BoundOrphanProcess::capture(pid, start)?;
    let mut system = System::new().map_err(|error| error.to_string())?;
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::everything().without_tasks(),
    );
    let process = system
        .process(sysinfo::Pid::from_u32(pid))
        .ok_or("orphan no longer exists")?;
    if crate::process::host_process_start_time(pid) != Some(start)
        || process.cwd() != Some(Path::new(expected_cwd))
        || process
            .exe()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_none_or(|name| !matches!(name, "codex" | "claude" | "codex.exe" | "claude.exe"))
    {
        return Err("orphan executable, cwd, or process generation changed".into());
    }
    let mut descendants = vec![pid];
    loop {
        let previous = descendants.len();
        for child in system.processes().values() {
            if child
                .parent()
                .is_some_and(|parent| descendants.contains(&parent.as_u32()))
                && !descendants.contains(&child.pid().as_u32())
            {
                descendants.push(child.pid().as_u32());
            }
        }
        if descendants.len() == previous {
            break;
        }
    }
    let identities: Vec<_> = descendants
        .into_iter()
        .filter_map(|child| {
            if child == pid {
                return Some((pid, start));
            }
            let snapshot = system.process(sysinfo::Pid::from_u32(child))?;
            if crate::process::host_process_start_epoch_secs(child) != Some(snapshot.start_time())
                || crate::process::host_process_start_time(pid) != Some(start)
                || !crate::process::is_descendant_of(child, pid).unwrap_or(false)
            {
                return None;
            }
            let identity = crate::process::snapshot_process_start_identity(child, snapshot);
            (identity > 0).then_some((child, identity))
        })
        .collect();
    let mut failures = Vec::new();
    for (child, identity) in identities.iter().rev() {
        if crate::process::host_process_start_time(*child) != Some(*identity) {
            continue;
        }
        if *child != pid
            && (crate::process::host_process_start_time(pid) != Some(start)
                || !crate::process::is_descendant_of(*child, pid).unwrap_or(false))
        {
            failures.push(*child);
            continue;
        }
        let result = if *child == pid {
            root.terminate()
        } else {
            BoundOrphanProcess::capture(*child, *identity).and_then(|process| process.terminate())
        };
        if result.is_err() {
            failures.push(*child);
        }
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let survivors = loop {
        let survivors: Vec<_> = identities
            .iter()
            .filter(|(child, identity)| {
                crate::process::host_process_start_time(*child) == Some(*identity)
            })
            .map(|(child, _)| *child)
            .collect();
        if survivors.is_empty() || std::time::Instant::now() >= deadline {
            break survivors;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    if !failures.is_empty() || !survivors.is_empty() {
        return Err(format!("process-tree termination incomplete; signal failures={failures:?}, remaining pids={survivors:?}; re-inventory before retry"));
    }
    Ok(json!({"terminated_pid":pid,"started_at":start}))
}

fn config_error(message: impl Into<String>) -> SpecOpsError {
    SpecOpsError::from(ApiError::Network(message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_core::process::{resolved_command, ProcessPlanRequest};
    use serde_json::json;

    struct OwnedChild(std::process::Child);

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    #[cfg(any(target_os = "linux", windows))]
    fn waiting_child() -> OwnedChild {
        #[cfg(target_os = "linux")]
        let mut command = {
            let mut command = resolved_command(ProcessPlanRequest::new("/bin/sh")).unwrap();
            command.args(["-c", "read ignored"]);
            command
        };
        #[cfg(windows)]
        let mut command = {
            let mut command = resolved_command(ProcessPlanRequest::new("cmd.exe")).unwrap();
            command.args(["/D", "/Q", "/C", "set /p gwt_test_input="]);
            command
        };
        OwnedChild(
            command
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn orphan_process_handle_rejects_wrong_generation_without_signalling() {
        let mut child = waiting_child();
        let pid = child.0.id();
        let started_at = crate::process::host_process_start_time(pid).unwrap();

        let result = BoundOrphanProcess::capture(pid, started_at + 1);

        assert!(result.err().unwrap().contains("generation changed"));
        assert!(child.0.try_wait().unwrap().is_none());
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn orphan_process_handle_terminates_the_captured_child() {
        let mut child = waiting_child();
        let pid = child.0.id();
        let started_at = crate::process::host_process_start_time(pid).unwrap();
        let process = BoundOrphanProcess::capture(pid, started_at).unwrap();

        process.terminate().unwrap();

        assert!(!child.0.wait().unwrap().success());
    }

    #[cfg(unix)]
    fn selected_orphan(worktree: &Path) -> (OwnedChild, Value, FleetTarget) {
        std::fs::create_dir_all(worktree).unwrap();
        crate::cli::trusted_store::init_git_repo_with_origin(worktree);
        let mut session =
            gwt_agent::Session::new(worktree, "work/orphan", gwt_agent::AgentId::Codex);
        session.id = "orphan-session".into();
        session.save(&gwt_core::paths::gwt_sessions_dir()).unwrap();
        let executable = worktree.join("codex");
        std::fs::copy("/bin/cat", &executable).unwrap();
        #[cfg(target_os = "macos")]
        {
            // Relocated system binaries retain a platform signature that can
            // make macOS kill the fixture before the cleanup under test.
            let signed = resolved_command(ProcessPlanRequest::new("/usr/bin/codesign"))
                .unwrap()
                .args(["--force", "--sign", "-"])
                .arg(&executable)
                .output()
                .unwrap();
            assert!(signed.status.success(), "{signed:?}");
        }
        let mut child = OwnedChild(
            resolved_command(ProcessPlanRequest::new(&executable))
                .unwrap()
                .current_dir(worktree)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
        use std::io::{Read, Write};
        child
            .0
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"ready\n")
            .unwrap();
        let mut ready = [0_u8; 6];
        child
            .0
            .stdout
            .as_mut()
            .unwrap()
            .read_exact(&mut ready)
            .unwrap();
        assert_eq!(&ready, b"ready\n");
        let pid = child.0.id();
        let started_at = crate::process::host_process_start_time(pid).unwrap();
        let cwd = worktree.canonicalize().unwrap();
        let row = json!({"worktree_path":cwd,"owner_number":4179,"session_id":session.id,
            "process":{"pid":pid,"started_at":started_at,"cwd":cwd,"worktree_path":cwd}});
        let target = serde_json::from_value(json!({
            "id":format!("process:{pid}"),"pid":pid,"started_at":started_at,
            "session_id":session.id,
        }))
        .unwrap();
        (child, row, target)
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn orphan_termination_refuses_without_a_stable_process_handle() {
        let temporary = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temporary.path().join("home"));
        let (mut child, row, target) = selected_orphan(&temporary.path().join("repo"));

        let result = terminate_orphan(&row, &target);

        assert!(child.0.try_wait().unwrap().is_none(), "{result:?}");
        let reason = result.unwrap_err();
        assert!(reason.contains("stable process handle"), "{reason}");
    }

    #[cfg(unix)]
    #[test]
    fn orphan_termination_protects_execution_acquired_after_selection() {
        let temporary = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temporary.path().join("home"));
        let worktree = temporary.path().join("repo");
        let (mut child, row, target) = selected_orphan(&worktree);
        crate::cli::execution_state::materialize_at_launch(
            &worktree,
            crate::cli::execution_state::ExecutionOwnerKind::Spec,
            4179,
            target.session_id.as_deref().unwrap(),
            "gwt-execute",
            false,
        )
        .unwrap();

        let result = terminate_orphan(&row, &target);

        assert!(child.0.try_wait().unwrap().is_none(), "{result:?}");
        let reason = result.unwrap_err();
        assert!(reason.contains("execution holder"), "{reason}");
    }

    #[cfg(unix)]
    #[test]
    fn orphan_termination_protects_claim_acquired_after_selection() {
        let temporary = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(temporary.path().join("home"));
        let worktree = temporary.path().join("repo");
        let (mut child, row, target) = selected_orphan(&worktree);
        crate::issue_monitor::mutate_issue_monitor_prefs(
            &crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&worktree),
            |prefs| {
                prefs.launched_claims.insert(4179, "new-claim".into());
                prefs.launch_bindings.insert("old-window".into(), 4179);
                prefs.launch_confirmations.insert(
                    4179,
                    crate::issue_monitor::IssueMonitorLaunchConfirmation {
                        window_id: "old-window".into(),
                        claim_id: Some("new-claim".into()),
                        tier: None,
                        delivery_id: None,
                        confirmed_at: "2026-10-23T00:00:00Z".into(),
                    },
                );
            },
        )
        .unwrap();

        let result = terminate_orphan(&row, &target);

        assert!(child.0.try_wait().unwrap().is_none(), "{result:?}");
        let reason = result.unwrap_err();
        assert!(reason.contains("claim holder"), "{reason}");

        #[cfg(target_os = "macos")]
        {
            crate::issue_monitor::mutate_issue_monitor_prefs(
                &crate::issue_monitor::issue_monitor_prefs_path_for_repo_path(&worktree),
                |prefs| {
                    prefs.launched_claims.clear();
                },
            )
            .unwrap();
            let reason = terminate_orphan(&row, &target).unwrap_err();
            assert!(reason.contains("stable process handle"), "{reason}");
            assert!(child.0.try_wait().unwrap().is_none());
        }
    }

    #[test]
    fn fleet_disposition_unknown_refuses_entire_batch_before_side_effects() {
        let rows = vec![
            json!({"id":"pane:a","delivery_state":"delivered"}),
            json!({"id":"pane:b","delivery_state":"unknown"}),
        ];
        let targets = vec![target("pane:a"), target("pane:b")];
        assert!(preflight(&rows, &targets, false)
            .unwrap_err()
            .contains("unknown"));
        assert!(preflight(&rows, &targets, true).is_ok());
    }

    fn target(id: &str) -> FleetTarget {
        serde_json::from_value(json!({"id":id,"session_id":"s","child_pid":1,"child_started_at":2}))
            .unwrap()
    }

    #[test]
    fn fleet_disposition_rejects_replaced_session_runtime_and_claim_holder() {
        let expected = target("pane:a");
        let mut row = json!({"id":"pane:a","session_id":"s", "claim_holder":false,
            "pane":{"child_pid":1,"child_started_at":2}});
        assert!(validate_target(&row, &expected).is_ok());
        row["session_id"] = json!("successor");
        assert!(validate_target(&row, &expected)
            .unwrap_err()
            .contains("Session"));
        row["session_id"] = json!("s");
        row["pane"]["child_started_at"] = json!(3);
        assert!(validate_target(&row, &expected)
            .unwrap_err()
            .contains("runtime"));
        row["pane"]["child_started_at"] = json!(2);
        row["claim_holder"] = json!(true);
        assert!(validate_target(&row, &expected)
            .unwrap_err()
            .contains("claim"));
    }
}
