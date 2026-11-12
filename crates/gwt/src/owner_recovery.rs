//! Diagnostics and explicit, identity-fenced recovery of an unresponsive daemon.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

/// Capture once when the endpoint is observed; never recapture before signaling.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerRecoveryTarget {
    pub pid: u32,
    pub started_at: u64,
}

impl OwnerRecoveryTarget {
    /// A descriptor cannot refer to a process born after it was written.
    /// This fences initial recovery when no successful Health reply exists yet.
    pub fn capture_endpoint(endpoint: &gwt_core::daemon::DaemonEndpoint) -> Result<Self, String> {
        let target = Self::capture(endpoint.pid)?;
        if !endpoint_could_describe_process(
            crate::process::host_process_start_epoch_secs(endpoint.pid),
            endpoint.updated_at_unix_ms,
        ) {
            return Err(
                "daemon PID was reused after endpoint publication or its birth is unavailable"
                    .into(),
            );
        }
        target.verify()?;
        Ok(target)
    }

    pub fn capture(pid: u32) -> Result<Self, String> {
        let started_at = crate::process::host_process_start_time(pid)
            .ok_or_else(|| "daemon process identity is unavailable".to_string())?;
        Ok(Self { pid, started_at })
    }

    fn verify(&self) -> Result<(), String> {
        if self.pid == 0
            || self.started_at == 0
            || crate::process::host_process_start_time(self.pid) != Some(self.started_at)
        {
            return Err("daemon process identity changed or is unavailable".into());
        }
        Ok(())
    }

    fn verify_for_termination(&self) -> Result<(), String> {
        if self.pid == std::process::id() {
            return Err("cannot terminate the current GUI host process".into());
        }
        self.verify()
    }
}

fn endpoint_could_describe_process(started_at: Option<u64>, updated_at_ms: i64) -> bool {
    let Ok(updated_at_secs) = u64::try_from(updated_at_ms / 1000) else {
        return false;
    };
    started_at.is_some_and(|started_at| started_at > 0 && started_at <= updated_at_secs)
}

/// Save evidence without signaling the daemon. Call from a background worker.
/// Failed platform probes are retained as diagnostics rather than silently skipped.
pub fn collect(target: &OwnerRecoveryTarget, project_key: &str) -> Result<PathBuf, String> {
    target.verify()?;
    let dir = gwt_core::paths::gwt_logs_dir()
        .join("owner-recovery")
        .join(format!("{}-{}", target.pid, uuid::Uuid::new_v4()));
    fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    let before = cpu_sample(target);
    let stack_path = dir.join("stack.txt");
    write_private(&stack_path, b"")?;
    let stack = collect_stack(target, &stack_path);
    std::thread::sleep(Duration::from_secs(1));
    let after = cpu_sample(target);
    write_private(&dir.join("stack.txt"), stack.as_bytes())?;
    let (warnings, log_sources) = recent_delivery_warnings(project_key)?;
    write_private(
        &dir.join("terminal-delivery-warnings.jsonl"),
        warnings.as_bytes(),
    )?;
    let manifest = serde_json::json!({
        "target": target,
        "collected_at": chrono::Utc::now().to_rfc3339(),
        "cpu_samples": [before, after],
        "stack_file": "stack.txt",
        "delivery_warnings_file": "terminal-delivery-warnings.jsonl",
        "delivery_log_sources": log_sources,
    });
    write_private(
        &dir.join("diagnostics.json"),
        serde_json::to_string_pretty(&manifest)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    )?;
    tracing::warn!(target: "gwt.owner.recovery", pid = target.pid, diagnostics = %dir.display(), "wedged daemon diagnostics saved; no signal sent");
    Ok(dir)
}

/// Stop an exact daemon only after an explicit user-confirmed recovery request.
/// The caller must require confirmation; diagnostics from this exact target are
/// mandatory. Existing endpoint resolution starts the replacement after exit.
pub fn terminate(target: &OwnerRecoveryTarget, diagnostics: &Path) -> Result<(), String> {
    terminate_with_grace(target, diagnostics, Duration::from_secs(5))
}

fn terminate_with_grace(
    target: &OwnerRecoveryTarget,
    diagnostics: &Path,
    grace: Duration,
) -> Result<(), String> {
    target.verify_for_termination()?;
    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(diagnostics.join("diagnostics.json")).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let recorded: OwnerRecoveryTarget =
        serde_json::from_value(manifest["target"].clone()).map_err(|error| error.to_string())?;
    if &recorded != target {
        return Err("diagnostics belong to another daemon process".into());
    }
    let log_path = diagnostics.join("termination.jsonl");
    record_termination(&log_path, target, "sigterm_requested")?;
    signal(target, false)?;
    if wait_for_exit(target, grace)? {
        record_termination(&log_path, target, "exited_after_sigterm")?;
        return Ok(());
    }
    // Persist the failed cooperative shutdown before escalation, not afterwards.
    record_termination(&log_path, target, "sigterm_did_not_exit_before_deadline")?;
    tracing::warn!(target: "gwt.owner.recovery", pid = target.pid, "wedged daemon did not exit after SIGTERM; escalating confirmed recovery to SIGKILL");
    signal(target, true)?;
    record_termination(&log_path, target, "sigkill_requested")?;
    if !wait_for_exit(target, Duration::from_secs(5))? {
        record_termination(&log_path, target, "still_alive_after_sigkill")?;
        return Err("daemon is still alive after forced termination".into());
    }
    record_termination(&log_path, target, "exited_after_sigkill")
}

fn signal(target: &OwnerRecoveryTarget, force: bool) -> Result<(), String> {
    target.verify_for_termination()?;
    #[cfg(unix)]
    {
        let pid = libc::pid_t::try_from(target.pid).map_err(|error| error.to_string())?;
        // SAFETY: a positive exact PID was verified immediately above. No
        // process group or name-based termination is used.
        if unsafe { libc::kill(pid, if force { libc::SIGKILL } else { libc::SIGTERM }) } != 0 {
            return Err(std::io::Error::last_os_error().to_string());
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, Signal, System};
        let mut system = System::new().map_err(|error| error.to_string())?;
        let pid = sysinfo::Pid::from_u32(target.pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let process = system.process(pid).ok_or("daemon process disappeared")?;
        if !force {
            // Windows has no POSIX SIGTERM; retain the cooperative-attempt
            // record and allow the same confirmed request's bounded escalation.
            let _ = process.kill_with(Signal::Term);
            return Ok(());
        }
        process
            .kill()
            .then_some(())
            .ok_or_else(|| "could not terminate daemon process".into())
    }
}

fn wait_for_exit(target: &OwnerRecoveryTarget, budget: Duration) -> Result<bool, String> {
    use sysinfo::{ProcessRefreshKind, ProcessStatus, ProcessesToUpdate, System};

    let deadline = Instant::now() + budget;
    let mut system = System::new().map_err(|error| error.to_string())?;
    let pid = sysinfo::Pid::from_u32(target.pid);
    loop {
        #[cfg(unix)]
        {
            // SAFETY: P_PID observes only this exact child. WNOHANG never
            // blocks and WNOWAIT leaves reaping to the owning supervisor.
            let exited = unsafe {
                let mut info: libc::siginfo_t = std::mem::zeroed();
                libc::waitid(
                    libc::P_PID,
                    target.pid as libc::id_t,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                ) == 0
                    && info.si_pid() == target.pid as libc::pid_t
            };
            if exited {
                return Ok(true);
            }
        }
        if crate::process::host_process_liveness(target.pid) == Some(false) {
            return Ok(true);
        }
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        // A supervisor-owned child can remain a zombie until this call returns
        // and the supervisor reaps it. kill(pid, 0) still succeeds for that
        // already exited process; it must not trigger SIGKILL escalation.
        if system.process(pid).is_some_and(|process| {
            matches!(
                process.status(),
                ProcessStatus::Zombie | ProcessStatus::Dead
            )
        }) {
            return Ok(true);
        }
        match crate::process::host_process_start_time(target.pid) {
            Some(started_at) if started_at != target.started_at => return Ok(true),
            Some(_) => {}
            None if Instant::now() < deadline => {
                // Exit may race the probes above. Retry observation without
                // signaling; the next waitid can see the now-unreaped child.
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            None => return Err("cannot determine daemon identity after signal".into()),
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn record_termination(
    path: &Path,
    target: &OwnerRecoveryTarget,
    event: &str,
) -> Result<(), String> {
    let mut file = private_options()
        .append(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    writeln!(
        file,
        "{}",
        serde_json::json!({"at": chrono::Utc::now().to_rfc3339(), "target": target, "event": event})
    )
    .and_then(|_| file.sync_all())
    .map_err(|error| error.to_string())
}

fn private_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
}

fn write_private(path: &Path, content: &[u8]) -> Result<(), String> {
    let mut file = private_options()
        .truncate(true)
        .open(path)
        .map_err(|error| error.to_string())?;
    file.write_all(content)
        .and_then(|_| file.sync_all())
        .map_err(|error| error.to_string())
}

fn cpu_sample(target: &OwnerRecoveryTarget) -> serde_json::Value {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
    let result = (|| {
        target.verify()?;
        let mut system = System::new().map_err(|error| error.to_string())?;
        let pid = sysinfo::Pid::from_u32(target.pid);
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing().with_cpu(),
        );
        let process = system.process(pid).ok_or("daemon process disappeared")?;
        Ok::<_, String>(process.accumulated_cpu_time())
    })();
    serde_json::json!({"at": chrono::Utc::now().to_rfc3339(), "cpu_time_ms": result.as_ref().ok(), "error": result.as_ref().err()})
}

fn collect_stack(target: &OwnerRecoveryTarget, stack_path: &Path) -> String {
    if let Err(error) = target.verify() {
        return error;
    }
    #[cfg(target_os = "macos")]
    {
        use gwt_core::process_console::{
            spawn_logged_blocking_with_deadline, ProcessConsoleHub, ProcessKind, SpawnOptions,
        };
        match spawn_logged_blocking_with_deadline(
            &ProcessConsoleHub::new(),
            ProcessKind::AgentBootstrap,
            "/usr/bin/sample",
            &[
                std::ffi::OsString::from(target.pid.to_string()),
                "1".into(),
                "1".into(),
                "-file".into(),
                stack_path.as_os_str().to_owned(),
            ],
            SpawnOptions::new("wedged daemon stack sample").forward_output(false),
            Instant::now() + Duration::from_secs(10),
        ) {
            Ok(output) => format!(
                "exit_code: {:?}\n{}\n{}\n{}",
                output.exit_code,
                fs::read_to_string(stack_path)
                    .unwrap_or_else(|error| format!("sample output unreadable: {error}")),
                output.stdout,
                output.stderr
            ),
            Err(error) => format!("stack sample failed: {error}"),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = stack_path;
        "Stack sampling is currently available on macOS only.".into()
    }
}

fn delivery_warnings(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let value: serde_json::Value = serde_json::from_str(line).ok()?;
            (value["target"] == "gwt.terminal.delivery" && value["level"] == "WARN")
                .then(|| gwt_core::process_console::redact_line(line))
        })
        .collect()
}

fn recent_delivery_warnings(project_key: &str) -> Result<(String, Vec<serde_json::Value>), String> {
    let project_hash =
        gwt_core::repo_hash::RepoHash::parse(project_key).map_err(|error| error.to_string())?;
    let mut warnings = Vec::new();
    let mut sources = Vec::new();
    for root in [
        gwt_core::paths::gwt_logs_dir(),
        gwt_core::paths::gwt_project_logs_dir(&project_hash),
    ] {
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) => {
                sources.push(serde_json::json!({"path": root, "error": error.to_string()}));
                continue;
            }
        };
        let mut files = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) if entry.file_name().to_string_lossy().starts_with("gwt.log.") => {
                    files.push(entry.path())
                }
                Ok(_) => {}
                Err(error) => {
                    sources.push(serde_json::json!({"path": root, "error": error.to_string()}))
                }
            }
        }
        files.sort();
        if files.is_empty() {
            sources.push(
                serde_json::json!({"path": root, "warning_count": 0, "note": "no log files"}),
            );
        }
        for path in files.iter().rev().take(2).rev() {
            let read = (|| {
                let mut file = File::open(path)?;
                let length = file.metadata()?.len();
                file.seek(SeekFrom::Start(length.saturating_sub(1024 * 1024)))?;
                let mut bytes = Vec::new();
                file.take(1024 * 1024).read_to_end(&mut bytes)?;
                Ok::<_, std::io::Error>(bytes)
            })();
            match read {
                Ok(bytes) => {
                    let found = delivery_warnings(&String::from_utf8_lossy(&bytes));
                    sources.push(serde_json::json!({"path": path, "warning_count": found.len(), "tail_limit_bytes": 1024 * 1024}));
                    warnings.extend(found);
                }
                Err(error) => {
                    sources.push(serde_json::json!({"path": path, "error": error.to_string()}))
                }
            }
        }
    }
    warnings.sort_by_cached_key(|line| {
        serde_json::from_str::<serde_json::Value>(line)
            .ok()
            .and_then(|value| value["timestamp"].as_str().map(str::to_owned))
            .unwrap_or_default()
    });
    Ok((
        warnings
            .into_iter()
            .rev()
            .take(100)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n"),
        sources,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_publication_cannot_precede_the_process_birth() {
        assert!(endpoint_could_describe_process(Some(100), 100_999));
        assert!(endpoint_could_describe_process(Some(99), 100_000));
        assert!(!endpoint_could_describe_process(Some(101), 100_999));
        assert!(!endpoint_could_describe_process(None, 100_999));
        assert!(!endpoint_could_describe_process(Some(0), 100_999));
        assert!(!endpoint_could_describe_process(Some(100), -1));
    }

    #[cfg(unix)]
    #[test]
    fn unreaped_child_exit_after_sigterm_does_not_escalate() {
        use std::process::Stdio;

        let mut child = gwt_core::process::hidden_command("/bin/sh")
            .args([
                "-c",
                "trap 'exit 0' TERM; printf ready; while :; do sleep 1; done",
            ])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = [0u8; 5];
        child.stdout.take().unwrap().read_exact(&mut ready).unwrap();
        let target = OwnerRecoveryTarget::capture(child.id()).unwrap();
        let root = tempfile::tempdir().unwrap();
        write_private(
            &root.path().join("diagnostics.json"),
            serde_json::to_string(&serde_json::json!({"target": target}))
                .unwrap()
                .as_bytes(),
        )
        .unwrap();

        // No concurrent waiter: the GUI supervisor cannot reap until recovery
        // returns and its next ensure_running call observes the child exit.
        let result = terminate_with_grace(&target, root.path(), Duration::from_secs(5));
        if result.is_err() {
            let _ = child.kill();
        }
        child.wait().unwrap();
        result.unwrap();
        let events = fs::read_to_string(root.path().join("termination.jsonl")).unwrap();
        assert!(events.contains("exited_after_sigterm"));
        assert!(!events.contains("sigkill_requested"));
        assert!(!events.contains("sigterm_did_not_exit_before_deadline"));
    }

    #[cfg(unix)]
    #[test]
    fn diagnostics_precede_confirmed_escalation_for_a_term_ignoring_process() {
        use std::process::Stdio;

        struct ChildCleanup {
            target: OwnerRecoveryTarget,
            reaper: Option<std::thread::JoinHandle<std::io::Result<std::process::ExitStatus>>>,
        }
        impl Drop for ChildCleanup {
            fn drop(&mut self) {
                if self.target.verify_for_termination().is_ok() {
                    let _ = signal(&self.target, true);
                }
                if let Some(reaper) = self.reaper.take() {
                    let _ = reaper.join();
                }
            }
        }

        let root = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(root.path());
        let mut child = gwt_core::process::hidden_command("/bin/sh")
            .args(["-c", "trap '' TERM; printf ready; exec sleep 60"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut ready = [0u8; 5];
        child.stdout.take().unwrap().read_exact(&mut ready).unwrap();
        assert_eq!(&ready, b"ready");
        let target = OwnerRecoveryTarget::capture(child.id()).unwrap();
        let _cleanup = ChildCleanup {
            target: target.clone(),
            reaper: Some(std::thread::spawn(move || child.wait())),
        };

        let project_key = "0123456789abcdef";
        let project_hash = gwt_core::repo_hash::RepoHash::parse(project_key).unwrap();
        let project_logs = gwt_core::paths::gwt_project_logs_dir(&project_hash);
        fs::create_dir_all(&project_logs).unwrap();
        fs::write(project_logs.join("gwt.log.2026-11-11"),
            "{\"level\":\"WARN\",\"target\":\"gwt.terminal.delivery\",\"fields\":{\"message\":\"project warning\"}}\n").unwrap();
        let diagnostics = collect(&target, project_key).unwrap();
        assert!(
            target.verify().is_ok(),
            "diagnosis must never kill the target"
        );
        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(diagnostics.join("diagnostics.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["cpu_samples"].as_array().unwrap().len(), 2);
        assert!(manifest["cpu_samples"][0]["cpu_time_ms"].is_number());
        assert!(manifest["cpu_samples"][1]["cpu_time_ms"].is_number());
        assert!(diagnostics.join("stack.txt").is_file());
        assert!(
            fs::read_to_string(diagnostics.join("terminal-delivery-warnings.jsonl"))
                .unwrap()
                .contains("project warning")
        );

        terminate_with_grace(&target, &diagnostics, Duration::from_millis(100)).unwrap();
        let events: Vec<String> = fs::read_to_string(diagnostics.join("termination.jsonl"))
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<serde_json::Value>(line).unwrap()["event"]
                    .as_str()
                    .unwrap()
                    .to_owned()
            })
            .collect();
        assert_eq!(
            events,
            [
                "sigterm_requested",
                "sigterm_did_not_exit_before_deadline",
                "sigkill_requested",
                "exited_after_sigkill"
            ]
        );
    }

    #[test]
    fn stale_identity_and_current_process_never_receive_a_signal() {
        let target = OwnerRecoveryTarget {
            pid: std::process::id(),
            started_at: crate::process::host_process_start_time(std::process::id()).unwrap(),
        };
        assert!(target.verify_for_termination().is_err());
        let stale = OwnerRecoveryTarget {
            started_at: target.started_at + 1,
            ..target
        };
        assert!(stale.verify().is_err());
    }

    #[test]
    fn delivery_excerpt_keeps_only_recent_delivery_warnings() {
        let input = concat!(
            "{\"level\":\"INFO\",\"target\":\"gwt.terminal.delivery\"}\n",
            "{\"level\":\"WARN\",\"target\":\"other\"}\n",
            "{\"level\":\"WARN\",\"target\":\"gwt.terminal.delivery\",\"fields\":{\"message\":\"kept\"}}\n"
        );
        assert_eq!(delivery_warnings(input).len(), 1);
        assert!(delivery_warnings(input)[0].contains("kept"));
    }
}
