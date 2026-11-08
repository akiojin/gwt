//! Read-only native ancestry proof for service-created verification workers.

use std::{
    ffi::{OsStr, OsString},
    sync::OnceLock,
};

/// A locator for a live original caller's membership broker, never authority
/// on its own. Every use checks native peer identity and Job membership.
pub const SOURCE_ENV: &str = "GWT_VERIFICATION_RECOVERY_SOURCE";

#[cfg(windows)]
pub use native::{checked_recovered_descendant_of, RecoverySource};

/// Capture only the locator once, before dispatch. Every relation check still
/// authenticates the live origin and broker; no positive result is cached.
pub fn capture_source() -> Option<&'static OsStr> {
    static SOURCE: OnceLock<Option<OsString>> = OnceLock::new();
    SOURCE
        .get_or_init(|| std::env::var_os(SOURCE_ENV))
        .as_deref()
}

pub fn is_recovered_descendant_of(pid: u32, ancestor: u32) -> bool {
    capture_source().is_some_and(|source| {
        checked_recovered_descendant_of(pid, ancestor, source).unwrap_or(false)
    })
}

#[cfg(not(windows))]
pub fn checked_recovered_descendant_of(
    _pid: u32,
    _ancestor: u32,
    _source: &OsStr,
) -> std::io::Result<bool> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "Windows recovery source cannot be authenticated on this platform",
    ))
}

#[cfg(windows)]
mod native {
    use std::{
        ffi::OsStr,
        io,
        os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        thread::{self, JoinHandle},
        time::{Duration, Instant},
    };

    use windows::{
        core::{Error as WindowsError, Owned, HRESULT, PCWSTR},
        Win32::{
            Foundation::{
                ERROR_BROKEN_PIPE, ERROR_INVALID_DATA, ERROR_NO_DATA, ERROR_PIPE_BUSY,
                ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, FILETIME, GENERIC_READ, HANDLE,
                WAIT_TIMEOUT,
            },
            Storage::FileSystem::{
                CreateFileW, ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_SHARE_MODE,
                OPEN_EXISTING, PIPE_ACCESS_DUPLEX, SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
            },
            System::{
                JobObjects::IsProcessInJob,
                Pipes::{
                    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
                    GetNamedPipeClientProcessId, GetNamedPipeServerProcessId, PeekNamedPipe,
                    PIPE_NOWAIT, PIPE_REJECT_REMOTE_CLIENTS,
                },
                Threading::{
                    GetCurrentProcess, GetProcessTimes, OpenProcess, WaitForSingleObject,
                    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                },
            },
        },
    };

    const POLL_INTERVAL: Duration = Duration::from_millis(100);
    const REPLY_TIMEOUT: Duration = Duration::from_secs(3);
    const MAGIC: &[u8; 8] = b"GWTREC01";
    const FRAME_SIZE: usize = 25;

    #[derive(serde::Serialize, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Descriptor {
        pid: u32,
        birth: u64,
        nonce: uuid::Uuid,
    }

    /// Serve only kernel-observed members of the exact recovery Job. No raw
    /// handle, requested PID, or arbitrary operation is accepted from a client.
    pub struct RecoverySource {
        descriptor: Descriptor,
        stop: Arc<AtomicBool>,
        worker: Option<JoinHandle<()>>,
    }

    impl RecoverySource {
        pub fn new(job: &gwt_core::process_tree::WindowsJobObject) -> io::Result<Self> {
            let query = job.duplicate_query_handle()?;
            let nonce = uuid::Uuid::new_v4();
            let name = pipe_name(nonce);
            let name: Vec<_> = name.encode_utf16().chain(Some(0)).collect();
            // SAFETY: the local name is NUL-terminated, unique and kept live.
            // This non-inheritable pipe rejects remote clients and cannot
            // silently share a name that another server already owns.
            let pipe = unsafe {
                CreateNamedPipeW(
                    PCWSTR(name.as_ptr()),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    FRAME_SIZE as u32,
                    1,
                    0,
                    None,
                )
            };
            if pipe.is_invalid() {
                return Err(io::Error::last_os_error());
            }
            // SAFETY: successful creation returned a fresh owned handle.
            let pipe = unsafe { OwnedHandle::from_raw_handle(pipe.0) };
            // SAFETY: the current process pseudo-handle is live.
            let birth = process_birth(unsafe { GetCurrentProcess() }).map_err(io::Error::other)?;
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = stop.clone();
            let worker = thread::Builder::new()
                .name("verification-source".into())
                .spawn(move || serve(pipe, query, nonce, &worker_stop))?;
            Ok(Self {
                descriptor: Descriptor {
                    pid: std::process::id(),
                    birth,
                    nonce,
                },
                stop,
                worker: Some(worker),
            })
        }

        pub fn descriptor(&self) -> String {
            serde_json::to_string(&self.descriptor).expect("fixed recovery source descriptor")
        }
    }

    impl Drop for RecoverySource {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(worker) = self.worker.take() {
                // Every server operation is nonblocking and every wait checks
                // stop; joining also closes its query handle before Job cleanup.
                let _ = worker.join();
            }
        }
    }

    fn pipe_name(nonce: uuid::Uuid) -> String {
        format!(r"\\.\pipe\gwt-verification-source-{nonce}")
    }

    fn raw(handle: &OwnedHandle) -> HANDLE {
        HANDLE(handle.as_raw_handle())
    }

    fn serve(pipe: OwnedHandle, job: OwnedHandle, nonce: uuid::Uuid, stop: &AtomicBool) {
        while !stop.load(Ordering::Acquire) {
            // SAFETY: the server handle is live and uses PIPE_NOWAIT.
            match unsafe { ConnectNamedPipe(raw(&pipe), None) } {
                Ok(()) => {}
                Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_CONNECTED.0) => {}
                Err(error) if error.code() == HRESULT::from_win32(ERROR_PIPE_LISTENING.0) => {
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }
                Err(error)
                    if error.code() == HRESULT::from_win32(ERROR_NO_DATA.0)
                        || error.code() == HRESULT::from_win32(ERROR_BROKEN_PIPE.0) =>
                {
                    // SAFETY: discard only this source's disconnected instance.
                    let _ = unsafe { DisconnectNamedPipe(raw(&pipe)) };
                    thread::sleep(POLL_INTERVAL);
                    continue;
                }
                Err(_) => break,
            }
            let mut frame = [0u8; FRAME_SIZE];
            frame[..MAGIC.len()].copy_from_slice(MAGIC);
            frame[MAGIC.len()..FRAME_SIZE - 1].copy_from_slice(nonce.as_bytes());
            frame[FRAME_SIZE - 1] =
                u8::from(client_is_member(raw(&pipe), raw(&job)).unwrap_or(false));
            let mut written = 0;
            // SAFETY: the fixed buffer is live. PIPE_NOWAIT prevents a client
            // that does not read from blocking this source's cleanup.
            let sent = unsafe { WriteFile(raw(&pipe), Some(&frame), Some(&mut written), None) };
            if sent.is_ok() && written == FRAME_SIZE as u32 {
                let deadline = Instant::now() + REPLY_TIMEOUT;
                // Disconnecting immediately would discard an unread frame.
                // Wait only for peer close, without accepting request payloads.
                while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
                    // SAFETY: Peek on this duplex nonblocking server cannot wait
                    // for input; a closed client ends the connection.
                    if unsafe { PeekNamedPipe(raw(&pipe), None, 0, None, None, None) }.is_err() {
                        break;
                    }
                    thread::sleep(POLL_INTERVAL);
                }
            }
            // SAFETY: this exact server owns the instance being disconnected.
            let _ = unsafe { DisconnectNamedPipe(raw(&pipe)) };
        }
    }

    fn client_is_member(pipe: HANDLE, job: HANDLE) -> windows::core::Result<bool> {
        let mut pid = 0;
        // SAFETY: kernel-reported peer identity, never a client-supplied PID.
        unsafe { GetNamedPipeClientProcessId(pipe, &mut pid)? };
        if pid == 0 {
            return Ok(false);
        }
        // SAFETY: retain this exact native process through membership readback.
        let process = unsafe {
            Owned::new(OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )?)
        };
        let mut member = windows::core::BOOL::default();
        // SAFETY: both query handles and the output remain live.
        unsafe { IsProcessInJob(*process, Some(job), &mut member)? };
        let mut after = 0;
        // SAFETY: recheck this same connected pipe after native membership.
        unsafe { GetNamedPipeClientProcessId(pipe, &mut after)? };
        // SAFETY: retained query/synchronize handle pins the process incarnation.
        Ok(member.as_bool()
            && after == pid
            && unsafe { WaitForSingleObject(*process, 0) } == WAIT_TIMEOUT)
    }

    /// A worker can ask only about itself. The environment only locates the
    /// broker; positive native peer, origin and membership evidence is required.
    pub fn checked_recovered_descendant_of(
        pid: u32,
        ancestor: u32,
        locator: &OsStr,
    ) -> io::Result<bool> {
        if pid != std::process::id() || ancestor == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "recovery source checks only the current process",
            ));
        }
        let invalid = || {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid recovery source locator",
            )
        };
        let locator = locator.to_str().ok_or_else(invalid)?;
        let source: Descriptor = serde_json::from_str(locator).map_err(|_| invalid())?;
        if source.pid == 0 || source.birth == 0 || source.nonce.is_nil() {
            return Err(invalid());
        }
        let origin = authenticated_origin(&source).map_err(io::Error::other)?;
        // The descriptor's ancestry is meaningful only after this exact broker
        // positively proves the caller's membership. Keep the native origin
        // handle pinned throughout that final relation check.
        let related = source.pid == ancestor
            || super::super::is_descendant_of(source.pid, ancestor).map_err(io::Error::other)?;
        // SAFETY: the retained origin has synchronization permission.
        if unsafe { WaitForSingleObject(*origin, 0) } != WAIT_TIMEOUT {
            return Err(io::Error::other("recovery source origin is no longer live"));
        }
        Ok(related)
    }

    fn unavailable() -> WindowsError {
        WindowsError::from_hresult(HRESULT::from_win32(ERROR_INVALID_DATA.0))
    }

    fn authenticated_origin(source: &Descriptor) -> windows::core::Result<Owned<HANDLE>> {
        // SAFETY: retain the exact origin, with read-only process rights, until
        // the server identity and response have both been rechecked.
        let origin = unsafe {
            Owned::new(OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                source.pid,
            )?)
        };
        // SAFETY: the origin has SYNCHRONIZE for this nonblocking liveness probe.
        if process_birth(*origin)? != source.birth
            || unsafe { WaitForSingleObject(*origin, 0) } != WAIT_TIMEOUT
        {
            return Err(unavailable());
        }
        let name: Vec<_> = pipe_name(source.nonce)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let deadline = Instant::now() + REPLY_TIMEOUT;
        let pipe = loop {
            // SAFETY: only a fixed local UUID name is used. Identification SQOS
            // prevents even a forged server from impersonating this caller.
            match unsafe {
                CreateFileW(
                    PCWSTR(name.as_ptr()),
                    GENERIC_READ.0,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                    None,
                )
            } {
                // SAFETY: successful open returned a new owned pipe handle.
                Ok(pipe) => break unsafe { OwnedHandle::from_raw_handle(pipe.0) },
                Err(error)
                    if error.code() == HRESULT::from_win32(ERROR_PIPE_BUSY.0)
                        && Instant::now() < deadline =>
                {
                    thread::sleep(POLL_INTERVAL)
                }
                Err(error) => return Err(error),
            }
        };
        let mut server = 0;
        // SAFETY: the live client handle identifies its native server process.
        unsafe { GetNamedPipeServerProcessId(raw(&pipe), &mut server)? };
        if server != source.pid {
            return Err(unavailable());
        }
        loop {
            let mut available = 0;
            // SAFETY: only this thread reads this handle; Peek never consumes
            // bytes. ReadFile is called only after a complete fixed frame exists.
            unsafe { PeekNamedPipe(raw(&pipe), None, 0, None, Some(&mut available), None)? };
            if available == FRAME_SIZE as u32 {
                break;
            }
            if available > FRAME_SIZE as u32 || Instant::now() >= deadline {
                return Err(unavailable());
            }
            thread::sleep(POLL_INTERVAL);
        }
        let mut frame = [0u8; FRAME_SIZE];
        let mut read = 0;
        // SAFETY: the full fixed frame is already buffered and this is its only
        // reader, so this synchronous read cannot wait for additional bytes.
        unsafe { ReadFile(raw(&pipe), Some(&mut frame), Some(&mut read), None)? };
        let mut after = 0;
        // SAFETY: recheck the connected server and retained origin after reading.
        unsafe { GetNamedPipeServerProcessId(raw(&pipe), &mut after)? };
        if read != FRAME_SIZE as u32
            || after != source.pid
            || process_birth(*origin)? != source.birth
            || unsafe { WaitForSingleObject(*origin, 0) } != WAIT_TIMEOUT
            || &frame[..MAGIC.len()] != MAGIC
            || &frame[MAGIC.len()..FRAME_SIZE - 1] != source.nonce.as_bytes()
            || frame[FRAME_SIZE - 1] != 1
        {
            return Err(unavailable());
        }
        Ok(origin)
    }

    pub(super) fn process_birth(process: HANDLE) -> windows::core::Result<u64> {
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: the live query handle and exact writable buffers are valid.
        unsafe { GetProcessTimes(process, &mut created, &mut exited, &mut kernel, &mut user)? };
        Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::{
        io::Read,
        os::windows::io::AsRawHandle,
        process::{Child, Stdio},
    };

    use super::native::process_birth;
    use super::*;
    use gwt_core::{
        process::{resolved_command, ProcessPlanRequest},
        process_tree::WindowsJobObject,
        test_support::{env_lock, ScopedEnvVar},
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    struct OwnedChild(Child);

    impl Drop for OwnedChild {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn probe(source: &str, job: &mut WindowsJobObject) -> bool {
        let mut command = resolved_command(ProcessPlanRequest::new(
            std::env::current_exe().expect("test binary"),
        ))
        .unwrap();
        command
            .args([
                "--exact",
                "process::windows_recovery::tests::member_fixture",
                "--nocapture",
            ])
            .env("GWT_RECOVERY_MEMBER_FIXTURE", "1")
            .env(SOURCE_ENV, source)
            .env("GWT_RECOVERY_ANCESTOR", std::process::id().to_string())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        WindowsJobObject::configure_suspended(&mut command);
        let mut child = OwnedChild(command.spawn().unwrap());
        job.assign_and_resume(child.0.id()).unwrap();
        assert!(child.0.wait().unwrap().success());
        let mut output = String::new();
        child
            .0
            .stdout
            .take()
            .unwrap()
            .read_to_string(&mut output)
            .unwrap();
        let result = output
            .lines()
            .find_map(|line| line.split_once("RECOVERY_MEMBER=").map(|(_, value)| value));
        result.expect("native member result").parse().unwrap()
    }

    #[test]
    fn member_fixture() {
        if std::env::var_os("GWT_RECOVERY_MEMBER_FIXTURE").is_some() {
            let _env = env_lock()
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let _source = std::env::var_os("GWT_RECOVERY_CLAIM_CLIENT").map(|_| {
                let mut forged: serde_json::Value =
                    serde_json::from_str(&std::env::var(SOURCE_ENV).unwrap()).unwrap();
                forged["pid"] = serde_json::json!(std::process::id());
                forged["birth"] =
                    serde_json::json!(process_birth(unsafe { GetCurrentProcess() }).unwrap());
                ScopedEnvVar::set(SOURCE_ENV, forged.to_string())
            });
            let ancestor = std::env::var("GWT_RECOVERY_ANCESTOR")
                .unwrap()
                .parse()
                .unwrap();
            let member = is_recovered_descendant_of(std::process::id(), ancestor);
            {
                let _unset = ScopedEnvVar::unset(SOURCE_ENV);
                assert_eq!(
                    is_recovered_descendant_of(std::process::id(), ancestor),
                    member,
                    "removing the environment locator must not change captured source identity"
                );
            }
            {
                let _replacement = ScopedEnvVar::set(SOURCE_ENV, "malformed replacement");
                assert_eq!(
                    is_recovered_descendant_of(std::process::id(), ancestor),
                    member,
                    "replacing the environment locator must not change captured source identity"
                );
            }
            let source = capture_source().unwrap();
            let checked = checked_recovered_descendant_of(std::process::id(), ancestor, source);
            if member {
                assert!(checked.unwrap());
                assert!(
                    !checked_recovered_descendant_of(
                        std::process::id(),
                        std::process::id(),
                        source
                    )
                    .unwrap(),
                    "authenticated source origin is unrelated to its child client as ancestor"
                );
            } else {
                assert!(
                    checked.is_err(),
                    "unproven membership must remain an error for revocation"
                );
            }
            println!("RECOVERY_MEMBER={member}");
        }
    }

    #[test]
    fn broker_requires_live_origin_and_exact_native_job_membership() {
        let _env = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut job = WindowsJobObject::new().unwrap();
        let source = RecoverySource::new(&job).unwrap();
        let descriptor = source.descriptor();
        assert!(probe(&descriptor, &mut job));
        let mut foreign = WindowsJobObject::new().unwrap();
        assert!(!probe(&descriptor, &mut foreign));
        {
            let _claim = ScopedEnvVar::set("GWT_RECOVERY_CLAIM_CLIENT", "1");
            assert!(
                !probe(&descriptor, &mut job),
                "native server PID must match the retained origin"
            );
        }
        let mut forged: serde_json::Value = serde_json::from_str(&descriptor).unwrap();
        forged["birth"] = serde_json::json!(forged["birth"].as_u64().unwrap() + 1);
        assert!(!probe(&forged.to_string(), &mut job));
        let mut forged: serde_json::Value = serde_json::from_str(&descriptor).unwrap();
        forged["nonce"] = serde_json::json!(uuid::Uuid::new_v4());
        assert!(!probe(&forged.to_string(), &mut job));
        assert!(!probe("malformed", &mut job));
        drop(source);
        assert!(!probe(&descriptor, &mut job));
    }

    #[test]
    fn foreign_query_handle_cannot_become_a_recovery_source() {
        let _env = env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut foreign = WindowsJobObject::new().unwrap();
        // A legitimate observer may transiently hold another pane's query-only
        // handle. Locating that handle must never grant recovery authority.
        let query = foreign.duplicate_query_handle().unwrap();
        let descriptor = serde_json::json!({
            "pid": std::process::id(),
            "birth": process_birth(unsafe { GetCurrentProcess() }).unwrap(),
            "handle": query.as_raw_handle() as usize,
        });
        assert!(!probe(&descriptor.to_string(), &mut foreign));
    }
}
