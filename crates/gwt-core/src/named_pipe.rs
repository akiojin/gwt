//! Windows named-pipe peer identity (Issue #3526).
//!
//! The pipe namespace is machine-global, so a deterministic pipe name can be
//! squatted by another local account before the real daemon binds it. The
//! endpoint file names the daemon's pid; a client compares that pid with the
//! pipe's actual server process before it sends the handshake (and with it
//! the auth token). Unix does not need this: the socket lives in the
//! owner-only runtime directory.

#[cfg(windows)]
use std::os::windows::io::RawHandle;

/// Process id of the server behind a connected named-pipe client handle.
///
/// `None` when the handle is not a named pipe or the query fails; callers
/// treat that as "identity unknown" and refuse to send credentials.
#[cfg(windows)]
pub fn named_pipe_server_process_id(handle: RawHandle) -> Option<u32> {
    use windows::Win32::{Foundation::HANDLE, System::Pipes::GetNamedPipeServerProcessId};

    let mut server_pid: u32 = 0;
    // SAFETY: `handle` is a live handle owned by the caller for the duration
    // of this call, and `server_pid` is a valid out-pointer.
    let queried = unsafe { GetNamedPipeServerProcessId(HANDLE(handle), &mut server_pid) };
    queried.ok().map(|()| server_pid)
}
