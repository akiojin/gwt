use std::{
    collections::{BTreeMap, HashMap},
    ffi::{CStr, OsStr},
    io,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use super::{collect_cwd_observation, AgentSwitchCwdObservation};

const NO_TTY: libc::dev_t = -1;
const UNKNOWN_UID: libc::uid_t = libc::uid_t::MAX;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
#[repr(C)]
struct ProcessInfo {
    started_at: u64,
    started_at_micros: i64,
    pid: libc::pid_t,
    parent: libc::pid_t,
    uid: libc::uid_t,
    tty: libc::dev_t,
    zombie: u32,
}

unsafe extern "C" {
    fn gwt_switch_process_snapshot(
        context: *mut libc::c_void,
        emit: unsafe extern "C" fn(*mut libc::c_void, *const ProcessInfo),
    ) -> libc::c_int;
    fn gwt_switch_process_info(pid: libc::pid_t, out: *mut ProcessInfo) -> libc::c_int;
}

fn snapshot() -> (Vec<ProcessInfo>, bool) {
    unsafe extern "C" fn append(context: *mut libc::c_void, process: *const ProcessInfo) {
        // SAFETY: the synchronous C caller passes back our Vec and one initialized record.
        unsafe { (*context.cast::<Vec<ProcessInfo>>()).push(*process) };
    }

    let mut records: Vec<ProcessInfo> = Vec::new();
    // SAFETY: the native loop borrows the Vec exclusively until it finishes.
    let complete = unsafe { gwt_switch_process_snapshot((&raw mut records).cast(), append) } == 1;
    (records, complete)
}

fn still_current(process: &ProcessInfo) -> bool {
    let mut current = ProcessInfo::default();
    // SAFETY: the native query writes one initialized repr(C) record on success.
    let read = unsafe { gwt_switch_process_info(process.pid, &mut current) };
    read == 1 && current == *process
}

fn session_id(process: &ProcessInfo) -> Option<libc::pid_t> {
    // SAFETY: getsid observes the session id of this positive PID.
    let sid = unsafe { libc::getsid(process.pid) };
    // Recheck incarnation and relationship metadata after the separate SID read.
    (sid > 0 && process.started_at > 0 && still_current(process)).then_some(sid)
}

fn outside_source_session(
    records: &HashMap<libc::pid_t, ProcessInfo>,
    source: &ProcessInfo,
    observer_uid: libc::uid_t,
    source_sid: Option<libc::pid_t>,
    candidate: &ProcessInfo,
    candidate_sid: Option<libc::pid_t>,
) -> bool {
    if candidate.uid == UNKNOWN_UID
        || source.uid == UNKNOWN_UID
        || candidate.uid == observer_uid
        || candidate.uid == source.uid
        || source_sid.is_none()
        || candidate_sid.is_none()
        || source_sid == candidate_sid
        || (source.tty != NO_TTY && source.tty == candidate.tty)
    {
        return false;
    }
    let mut parent = candidate.parent;
    for _ in 0..records.len() {
        if parent == 0 {
            return true;
        }
        if parent == source.pid {
            return false;
        }
        let Some(record) = records.get(&parent) else {
            return false;
        };
        parent = record.parent;
    }
    false
}

fn cwd(pid: libc::pid_t) -> Option<PathBuf> {
    // SAFETY: both the libc struct and all of its fields accept zero initialization.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of_val(&info) as libc::c_int;
    // SAFETY: the buffer matches the size and layout expected for this flavor.
    let read = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    let directory = &info.pvi_cdir;
    if read != size || directory.vip_vi.vi_stat.vst_dev == 0 {
        return None;
    }
    // SAFETY: vip_path is one contiguous, initialized array of C chars.
    let bytes = unsafe {
        std::slice::from_raw_parts(
            directory.vip_path.as_ptr().cast::<u8>(),
            std::mem::size_of_val(&directory.vip_path),
        )
    };
    let path = CStr::from_bytes_until_nul(bytes).ok();
    path.filter(|path| !path.is_empty())
        .map(|path| PathBuf::from(OsStr::from_bytes(path.to_bytes())))
}

pub(super) fn observe_checkpoint(
    worktree: &Path,
    source_pid: u32,
    source_started_at: u64,
) -> AgentSwitchCwdObservation {
    let (snapshot, complete) = snapshot();
    let captured = snapshot.len();
    let records: HashMap<_, _> = snapshot
        .into_iter()
        .map(|entry| (entry.pid, entry))
        .collect();
    let source = records.get(&(source_pid as libc::pid_t)).filter(|source| {
        source.pid > 0 && source_started_at != 0 && source.started_at == source_started_at
    });
    let source_sid = source.and_then(session_id);
    // SAFETY: geteuid only reads the current process's credentials.
    let observer_uid = unsafe { libc::geteuid() };
    let mut excluded_by_uid = BTreeMap::<_, usize>::new();
    let processes = records.values().filter_map(|process| {
        // PID 0 is the kernel, and a zombie has already released its cwd.
        if process.pid <= 0 || process.zombie != 0 {
            return None;
        }
        let path = cwd(process.pid);
        if path.is_none() {
            // A process that actually exited after enumeration has released its cwd.
            // SAFETY: signal 0 only checks existence/permissions for a positive PID.
            if unsafe { libc::kill(process.pid, 0) } == -1
                && io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
            {
                return None;
            }
            if process.uid != observer_uid
                && source.is_some_and(|source| {
                    outside_source_session(
                        &records,
                        source,
                        observer_uid,
                        source_sid,
                        process,
                        session_id(process),
                    )
                })
            {
                *excluded_by_uid.entry(process.uid).or_default() += 1;
                return None;
            }
        }
        Some((process.pid as u32, process.started_at, path))
    });
    let mut observation = collect_cwd_observation(
        worktree,
        processes,
        (
            captured,
            records.len(),
            complete && records.contains_key(&(std::process::id() as libc::pid_t)),
        ),
        Some((source_pid, source_started_at)),
    );
    observation.complete &= source.is_some_and(still_current);
    tracing::debug!(
        source_pid,
        complete = observation.complete,
        matching = observation.matching_pids.len(),
        unknown = observation.unknown_pids.len(),
        ?excluded_by_uid,
        "Switch cwd census"
    );
    observation
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_cwd_blocks_for_same_user_related_or_unobserved_processes() {
        let source = ProcessInfo {
            pid: 10,
            parent: 1,
            uid: 501,
            tty: 42,
            started_at: 100,
            ..ProcessInfo::default()
        };
        let unrelated = ProcessInfo {
            pid: 20,
            parent: 1,
            uid: 0,
            tty: NO_TTY,
            ..source
        };
        let middle = ProcessInfo {
            pid: 11,
            parent: source.pid,
            ..source
        };
        let records = [
            ProcessInfo {
                pid: 1,
                parent: 0,
                ..unrelated
            },
            source,
            middle,
        ]
        .into_iter()
        .map(|process| (process.pid, process))
        .collect();
        let exclude = |candidate, sid| {
            outside_source_session(&records, &source, 501, Some(10), &candidate, sid)
        };

        assert!(exclude(unrelated, Some(20)));
        assert!(!exclude(
            ProcessInfo {
                uid: 501,
                ..unrelated
            },
            Some(20)
        ));
        assert!(!exclude(
            ProcessInfo {
                parent: middle.pid,
                ..unrelated
            },
            Some(20)
        ));
        assert!(!exclude(unrelated, Some(10)));
        assert!(!exclude(
            ProcessInfo {
                tty: source.tty,
                ..unrelated
            },
            Some(20)
        ));
        assert!(!exclude(
            ProcessInfo {
                parent: 99,
                ..unrelated
            },
            Some(20)
        ));
        assert!(!exclude(unrelated, None));
        assert!(!outside_source_session(
            &records,
            &source,
            501,
            None,
            &unrelated,
            Some(20)
        ));
    }
}
