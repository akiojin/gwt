//! Conservative garbage collection of abandoned Host runtime sidecars (#5384).

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

use fs2::FileExt;

use crate::{with_session_path_lease, AgentStatus, SessionPathState, SessionRuntimeState};

/// Sidecar writers share this lease; GC takes it exclusively only for one
/// file's final validation/unlink. Always acquire the Session lease first.
pub(crate) fn runtime_publication_lock(sessions_dir: &Path) -> io::Result<fs::File> {
    fs::create_dir_all(sessions_dir)?;
    fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sessions_dir.join(".runtime-publication.lock"))
}

pub const RUNTIME_NAMESPACE_MIN_AGE: Duration = Duration::from_secs(24 * 60 * 60);
pub const RUNTIME_NAMESPACE_GC_INTERVAL: Duration = Duration::from_secs(60 * 60);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RuntimeGcReport {
    pub reclaimed: usize,
    pub retained: usize,
    pub reasons: BTreeMap<&'static str, usize>,
}

impl RuntimeGcReport {
    fn retain(&mut self, reason: &'static str) {
        self.retained += 1;
        *self.reasons.entry(reason).or_default() += 1;
    }
}

/// Collect only flat, readable, old namespaces with a decisively absent Host.
/// Durable Session metadata and recovery receipts are never removed. The PID
/// probe returns `None` for uncertainty, which always retains the namespace.
pub fn collect_stale_runtime_namespaces(
    sessions_dir: &Path,
    now: SystemTime,
    min_age: Duration,
    process_alive: impl Fn(u32) -> Option<bool>,
) -> io::Result<RuntimeGcReport> {
    let mut report = RuntimeGcReport::default();
    let root = sessions_dir.join("runtime");
    match fs::symlink_metadata(&root) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(error) => return Err(error),
        Ok(metadata) if !plain_directory(&metadata) => {
            report.retain("unknown_path");
            return Ok(report);
        }
        Ok(_) => {}
    }
    // GUI and project daemons share the same runtime root. Never race another
    // collector; ordinary Session writers retain their per-Session leases.
    let lease = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(sessions_dir.join(".runtime-gc.lock"))?;
    lease.try_lock_exclusive()?;
    for entry in fs::read_dir(&root)? {
        let entry = entry?;
        let path = entry.path();
        let pid = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<u32>().ok());
        let Some(pid) = pid.filter(|pid| *pid > 0) else {
            report.retain("unknown_namespace");
            continue;
        };
        let reason = match process_alive(pid) {
            Some(true) => Some("live_pid"),
            None => Some("unknown_pid"),
            Some(false) => None,
        };
        if let Some(reason) = reason {
            report.retain(reason);
            continue;
        }
        let files = match eligible_files(&path, sessions_dir, now, min_age, &process_alive) {
            Ok(files) => files,
            Err(reason) => {
                report.retain(reason);
                continue;
            }
        };
        // Recheck the Host and directory immediately before deleting. Delete
        // individual validated files, never recursively traverse a namespace.
        if process_alive(pid) != Some(false) {
            report.retain("pid_changed");
            continue;
        }
        if !fs::symlink_metadata(&path).is_ok_and(|metadata| plain_directory(&metadata)) {
            report.retain("unknown_path");
            continue;
        }
        let removal = files.iter().try_for_each(|file| {
            let id = file
                .file_stem()
                .and_then(|id| id.to_str())
                .ok_or("unknown_file")?;
            with_session_path_lease(sessions_dir, id, |state| {
                let result = (|| {
                    session_is_disposable(state)?;
                    let publication = runtime_publication_lock(sessions_dir)
                        .map_err(|_| "publication_unavailable")?;
                    publication
                        .try_lock_exclusive()
                        .map_err(|_| "publication_busy")?;
                    namespace_still_safe(&root, &path, pid, &process_alive)?;
                    sidecar_is_disposable(file, sessions_dir, now, min_age, &process_alive)?;
                    fs::remove_file(file).map_err(|_| "remove_failed")
                })();
                Ok(result)
            })
            .map_err(|_| "session_unavailable")?
        });
        let removal = removal.and_then(|()| {
            let publication =
                runtime_publication_lock(sessions_dir).map_err(|_| "publication_unavailable")?;
            publication
                .try_lock_exclusive()
                .map_err(|_| "publication_busy")?;
            namespace_still_safe(&root, &path, pid, &process_alive)?;
            if files.is_empty() {
                let metadata = fs::symlink_metadata(&path).map_err(|_| "unreadable")?;
                old_enough(&metadata, now, min_age)?;
            }
            fs::remove_dir(&path).map_err(|_| "namespace_changed")
        });
        match removal {
            Ok(()) => report.reclaimed += 1,
            Err(reason) => report.retain(reason),
        }
    }
    Ok(report)
}

fn namespace_still_safe(
    root: &Path,
    path: &Path,
    pid: u32,
    process_alive: &impl Fn(u32) -> Option<bool>,
) -> Result<(), &'static str> {
    for directory in [root, path] {
        if !fs::symlink_metadata(directory).is_ok_and(|metadata| plain_directory(&metadata)) {
            return Err("unknown_path");
        }
    }
    if process_alive(pid) != Some(false) {
        return Err("pid_changed");
    }
    Ok(())
}

fn eligible_files(
    path: &Path,
    sessions_dir: &Path,
    now: SystemTime,
    min_age: Duration,
    process_alive: &impl Fn(u32) -> Option<bool>,
) -> Result<Vec<PathBuf>, &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "unreadable")?;
    if !plain_directory(&metadata) {
        return Err("unknown_path");
    }
    old_enough(&metadata, now, min_age)?;
    let mut files = Vec::new();
    for entry in fs::read_dir(path).map_err(|_| "unreadable")? {
        let file = entry.map_err(|_| "unreadable")?.path();
        sidecar_is_disposable(&file, sessions_dir, now, min_age, process_alive)?;
        let id = file
            .file_stem()
            .and_then(|id| id.to_str())
            .ok_or("unknown_file")?;
        let state = crate::inspect_session_path(&sessions_dir.join(format!("{id}.toml")));
        session_is_disposable(state)?;
        files.push(file);
    }
    Ok(files)
}

fn session_is_disposable(state: SessionPathState) -> Result<(), &'static str> {
    match state {
        SessionPathState::Missing => Ok(()),
        SessionPathState::Present(session)
            if !session.restore_window_on_startup && session.status == AgentStatus::Stopped =>
        {
            Ok(())
        }
        SessionPathState::Present(_) => Err("session_recovery"),
        SessionPathState::Error(_) => Err("session_unavailable"),
    }
}

fn sidecar_is_disposable(
    path: &Path,
    sessions_dir: &Path,
    now: SystemTime,
    min_age: Duration,
    process_alive: &impl Fn(u32) -> Option<bool>,
) -> Result<(), &'static str> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "unreadable")?;
    if !metadata.is_file() || linked(&metadata) || path.extension().is_none_or(|ext| ext != "json")
    {
        return Err("unknown_file");
    }
    old_enough(&metadata, now, min_age)?;
    let id = path
        .file_stem()
        .and_then(|id| id.to_str())
        .ok_or("unknown_file")?;
    crate::validate_session_id_path_component(id).map_err(|_| "unknown_file")?;
    for directory in [
        "active-launch-handshakes",
        "manual-handoffs",
        "execution-launch-recovery",
    ] {
        match fs::symlink_metadata(sessions_dir.join(directory).join(format!("{id}.json"))) {
            Ok(_) => return Err("session_recovery"),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => return Err("session_unavailable"),
        }
    }
    let runtime = SessionRuntimeState::load(path).map_err(|_| "unreadable")?;
    for updated in [runtime.updated_at, runtime.last_activity_at] {
        if !now
            .duration_since(updated.into())
            .is_ok_and(|age| age >= min_age)
        {
            return Err("recent");
        }
    }
    if runtime.pending_discussion.is_some() {
        return Err("runtime_recovery");
    }
    match (runtime.child_pid, runtime.child_started_at) {
        (None, None) => {}
        (Some(pid), Some(started)) if pid > 0 && started > 0 => {
            if process_alive(pid) != Some(false) || process_group_may_exist(pid) {
                return Err("child_runtime");
            }
        }
        _ => return Err("unknown_child"),
    }
    Ok(())
}

fn old_enough(
    metadata: &fs::Metadata,
    now: SystemTime,
    min_age: Duration,
) -> Result<(), &'static str> {
    let modified = metadata.modified().map_err(|_| "unreadable")?;
    match now.duration_since(modified) {
        Ok(age) if age >= min_age => Ok(()),
        _ => Err("recent"),
    }
}

fn plain_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir() && !linked(metadata)
}

fn linked(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        // All reparse points, including directory junctions, fail closed.
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn process_group_may_exist(pid: u32) -> bool {
    #[cfg(unix)]
    {
        let Ok(pid) = libc::pid_t::try_from(pid) else {
            return true;
        };
        // SAFETY: signal zero observes the group without delivering a signal.
        (unsafe { libc::kill(-pid, 0) }) == 0
            || io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        false
    }
}
