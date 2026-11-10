//! Parent-only process observations for short-lived hook callers.

use std::io;

pub(super) fn is_descendant_of(pid: u32, ancestor: u32) -> Result<bool, sysinfo::Error> {
    #[cfg(windows)]
    let parents = windows_parent_ids()?;
    walk_parents(pid, ancestor, |pid| {
        #[cfg(windows)]
        return Ok(parents.get(&pid).copied());
        #[cfg(not(windows))]
        parent_id(pid)
    })
    .map_err(Into::into)
}

fn walk_parents(
    mut pid: u32,
    ancestor: u32,
    mut parent: impl FnMut(u32) -> io::Result<Option<u32>>,
) -> io::Result<bool> {
    for _ in 0..64 {
        let Some(next) = parent(pid)? else {
            return Ok(false);
        };
        if next == ancestor {
            return Ok(true);
        }
        if next <= 1 {
            return Ok(false);
        }
        pid = next;
    }
    Ok(false)
}

#[cfg(windows)]
fn windows_parent_ids() -> io::Result<std::collections::HashMap<u32, u32>> {
    use windows::{
        core::Owned,
        Win32::{
            Foundation::ERROR_NO_MORE_FILES,
            System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            },
        },
    };

    // SAFETY: the owned snapshot is closed on every path. ToolHelp exposes
    // parent IDs directly; opening every process to collect its metadata is
    // unnecessary for ancestry, and consumes the hook's aggregate deadline.
    let snapshot = unsafe { Owned::new(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?) };
    let mut entry = PROCESSENTRY32W {
        dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    let mut parents = std::collections::HashMap::new();
    // SAFETY: entry has the required native size and remains valid for each
    // call; the snapshot handle stays open until enumeration finishes.
    let mut result = unsafe { Process32FirstW(*snapshot, &mut entry) };
    loop {
        match result {
            Ok(()) => {
                parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
            }
            Err(error) if error.code() == ERROR_NO_MORE_FILES.to_hresult() => return Ok(parents),
            Err(error) => return Err(error.into()),
        }
        // SAFETY: same initialized entry and live owned snapshot as above.
        result = unsafe { Process32NextW(*snapshot, &mut entry) };
    }
}

#[cfg(target_os = "macos")]
fn parent_id(pid: u32) -> io::Result<Option<u32>> {
    let Ok(pid) = i32::try_from(pid) else {
        return Ok(None);
    };
    // SAFETY: proc_pidinfo receives exactly the native proc_bsdinfo buffer.
    let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as i32;
    let count =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size) };
    if count == size {
        return Ok(Some(info.pbi_ppid));
    }
    if count <= 0 {
        let error = io::Error::last_os_error();
        return if error.raw_os_error() == Some(libc::ESRCH)
            || error.kind() == io::ErrorKind::NotFound
        {
            Ok(None)
        } else {
            Err(error)
        };
    }
    Err(io::Error::new(
        io::ErrorKind::InvalidData,
        "short process parent observation",
    ))
}

#[cfg(target_os = "linux")]
fn parent_id(pid: u32) -> io::Result<Option<u32>> {
    let stat = match std::fs::read(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    parse_linux_parent(&stat).map(Some)
}

#[cfg(any(target_os = "linux", test))]
fn parse_linux_parent(stat: &[u8]) -> io::Result<u32> {
    stat.iter()
        .rposition(|byte| *byte == b')')
        .and_then(|end| {
            stat[end + 1..]
                .split(|byte| byte.is_ascii_whitespace())
                .filter(|field| !field.is_empty())
                .nth(1)
        })
        .and_then(|parent| std::str::from_utf8(parent).ok()?.parse().ok())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid process parent observation",
            )
        })
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn parent_id(_pid: u32) -> io::Result<Option<u32>> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancestry_reads_only_its_parent_chain_and_preserves_unknown() {
        let mut visited = Vec::new();
        assert!(walk_parents(40, 20, |pid| {
            visited.push(pid);
            Ok(Some(pid - 10))
        })
        .unwrap());
        assert_eq!(visited, [40, 30]);
        assert!(!walk_parents(40, 20, |_| Ok(None)).unwrap());
        let error =
            walk_parents(40, 20, |_| Err(io::ErrorKind::PermissionDenied.into())).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let mut visits = 0;
        assert!(!walk_parents(40, 20, |_| {
            visits += 1;
            Ok(Some(40))
        })
        .unwrap());
        assert_eq!(visits, 64);
        assert!(!walk_parents(40, 20, |_| Ok(Some(1))).unwrap());
    }

    #[test]
    fn linux_parent_parser_handles_names_with_spaces_and_parentheses() {
        assert_eq!(
            parse_linux_parent(b"40 (worker (hook) x) S 30 40 0").unwrap(),
            30
        );
        assert_eq!(
            parse_linux_parent(b"40 (worker\xff) S 30 40 0").unwrap(),
            30
        );
        assert!(parse_linux_parent(b"40 (worker) S nope").is_err());
        assert!(parse_linux_parent(b"40 (worker) S").is_err());
    }

    #[test]
    fn native_ancestry_matches_live_parent_and_rejects_missing_process() {
        use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
        let pid = std::process::id();
        let mut system = System::new().unwrap();
        system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[Pid::from_u32(pid)]),
            true,
            ProcessRefreshKind::nothing(),
        );
        let parent = system
            .process(Pid::from_u32(pid))
            .unwrap()
            .parent()
            .unwrap()
            .as_u32();
        assert!(crate::process::is_descendant_of(pid, parent).unwrap());
        assert!(!crate::process::is_descendant_of(u32::MAX, pid).unwrap());
        assert!(!crate::process::is_descendant_of(pid, pid).unwrap());
        assert!(!crate::process::is_descendant_of(0, pid).unwrap());
        assert!(!crate::process::is_descendant_of(pid, 0).unwrap());
    }
}
