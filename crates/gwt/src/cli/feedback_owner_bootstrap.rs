//! Private, secret-free bootstrap for the Windows feedback owner roles.

#[cfg(any(windows, test))]
use std::{io, time::Duration};

#[cfg(any(windows, test))]
use serde::{Deserialize, Serialize};
#[cfg(any(windows, test))]
use uuid::Uuid;

#[cfg(any(windows, test))]
const MAX_BOOTSTRAP_BYTES: usize = 4096;

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Role {
    Supervisor,
    Sender,
}

#[cfg(windows)]
impl Role {
    pub fn arg(self) -> &'static str {
        match self {
            Self::Supervisor => "--gwt-feedback-supervisor",
            Self::Sender => "--gwt-feedback-sender",
        }
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Clock {
    pub d0_ticks: i64,
    pub frequency: i64,
}

#[cfg(any(windows, test))]
impl Clock {
    pub fn remaining_work_at(self, now: i64, frequency: i64) -> io::Result<Duration> {
        self.remaining_at(now, frequency, 115)
    }

    pub fn remaining_exit_at(self, now: i64, frequency: i64) -> io::Result<Duration> {
        self.remaining_at(now, frequency, 120)
    }

    fn remaining_at(self, now: i64, frequency: i64, seconds: i64) -> io::Result<Duration> {
        if self.d0_ticks < 0
            || self.frequency <= 0
            || frequency != self.frequency
            || now < self.d0_ticks
        {
            return Err(invalid("invalid bootstrap clock"));
        }
        // Validate both fixed deadlines even when projecting only the work deadline.
        self.frequency
            .checked_mul(120)
            .and_then(|budget| self.d0_ticks.checked_add(budget))
            .ok_or_else(|| invalid("bootstrap clock overflow"))?;
        let ticks = self
            .frequency
            .checked_mul(seconds)
            .and_then(|budget| self.d0_ticks.checked_add(budget))
            .and_then(|deadline| deadline.checked_sub(now))
            .and_then(|remaining| remaining.checked_sub(1))
            .filter(|remaining| *remaining > 0)
            .ok_or_else(|| invalid("bootstrap deadline elapsed"))?;
        // Cross-thread/process QPC ordering has a one-tick uncertainty. Round down.
        let nanos = (ticks as u128)
            .checked_mul(1_000_000_000)
            .ok_or_else(|| invalid("bootstrap duration overflow"))?
            / self.frequency as u128;
        Ok(Duration::new(
            (nanos / 1_000_000_000) as u64,
            (nanos % 1_000_000_000) as u32,
        ))
    }
}

#[cfg(any(windows, test))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CreationIdentity {
    pub pid: u32,
    pub created: u64,
}

#[cfg(any(windows, test))]
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bootstrap {
    pub schema_version: u8,
    pub role: Role,
    #[serde(with = "uuid_text")]
    pub receipt_id: Uuid,
    #[serde(with = "uuid_text")]
    pub attempt_id: Uuid,
    #[serde(with = "uuid_text")]
    pub owner_id: Uuid,
    pub clock: Clock,
    #[serde(deserialize_with = "required_owner_identity")]
    pub owner_identity: Option<CreationIdentity>,
}

#[cfg(any(windows, test))]
fn required_owner_identity<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<CreationIdentity>, D::Error> {
    Option::deserialize(deserializer)
}

#[cfg(any(windows, test))]
mod uuid_text {
    use super::*;
    use serde::{Deserializer, Serializer};

    pub fn serialize<S: Serializer>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(id)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
        let text = String::deserialize(deserializer)?;
        Uuid::parse_str(&text).map_err(serde::de::Error::custom)
    }
}

#[cfg(any(windows, test))]
impl Bootstrap {
    pub fn supervisor(receipt_id: Uuid, attempt_id: Uuid, owner_id: Uuid, clock: Clock) -> Self {
        Self {
            schema_version: 1,
            role: Role::Supervisor,
            receipt_id,
            attempt_id,
            owner_id,
            clock,
            owner_identity: None,
        }
    }

    pub fn sender(&self, owner_identity: CreationIdentity) -> Self {
        Self {
            role: Role::Sender,
            owner_identity: Some(owner_identity),
            ..self.clone()
        }
    }

    pub fn encode(&self) -> io::Result<Vec<u8>> {
        self.validate(self.role)?;
        let bytes = serde_json::to_vec(self).map_err(|_| invalid("invalid bootstrap frame"))?;
        if bytes.len() > MAX_BOOTSTRAP_BYTES - 4 {
            return Err(invalid("bootstrap frame too large"));
        }
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8], role: Role) -> io::Result<Self> {
        if bytes.is_empty() || bytes.len() > MAX_BOOTSTRAP_BYTES - 4 {
            return Err(invalid("invalid bootstrap frame length"));
        }
        let frame: Self =
            serde_json::from_slice(bytes).map_err(|_| invalid("invalid bootstrap frame"))?;
        frame.validate(role)?;
        Ok(frame)
    }

    fn validate(&self, role: Role) -> io::Result<()> {
        if self.schema_version != 1
            || self.role != role
            || self.receipt_id.is_nil()
            || self.attempt_id.is_nil()
            || self.owner_id.is_nil()
            || (self.role == Role::Sender) != self.owner_identity.is_some()
            || self
                .owner_identity
                .is_some_and(|id| id.pid == 0 || id.created == 0)
        {
            return Err(invalid("invalid bootstrap binding"));
        }
        self.clock
            .remaining_work_at(self.clock.d0_ticks, self.clock.frequency)?;
        Ok(())
    }
}

#[cfg(any(windows, test))]
fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(windows)]
pub(super) use native::*;

#[cfg(windows)]
mod native {
    use super::*;
    use std::{
        ffi::OsStr,
        os::windows::{
            ffi::OsStrExt,
            io::{
                AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, IntoRawHandle, OwnedHandle,
            },
        },
        path::Path,
        time::Instant,
    };
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::{
            Foundation::{
                CloseHandle, DuplicateHandle, GetHandleInformation, SetHandleInformation,
                DUPLICATE_HANDLE_OPTIONS, FILETIME, GENERIC_READ, GENERIC_WRITE, HANDLE,
                HANDLE_FLAGS, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
                WAIT_TIMEOUT,
            },
            Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING,
            },
            System::{
                Memory::{
                    CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP_READ,
                    FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
                },
                Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
                Threading::{
                    CreateEventW, CreateProcessW, DeleteProcThreadAttributeList, GetCurrentProcess,
                    GetExitCodeProcess, GetProcessId, GetProcessTimes,
                    InitializeProcThreadAttributeList, ResetEvent, ResumeThread, SetEvent,
                    TerminateProcess, UpdateProcThreadAttribute, WaitForMultipleObjects,
                    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_SUSPENDED, EVENT_MODIFY_STATE,
                    EXTENDED_STARTUPINFO_PRESENT, LPPROC_THREAD_ATTRIBUTE_LIST,
                    PROCESS_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, STARTF_USESTDHANDLES, STARTUPINFOEXW,
                    SYNCHRONIZATION_SYNCHRONIZE,
                },
            },
        },
    };

    fn win_error(error: windows::core::Error) -> io::Error {
        io::Error::from_raw_os_error(error.code().0 & 0xffff)
    }

    fn raw(handle: BorrowedHandle<'_>) -> HANDLE {
        HANDLE(handle.as_raw_handle())
    }

    fn owned(handle: HANDLE) -> OwnedHandle {
        // SAFETY: every caller passes a newly created or duplicated owned handle.
        unsafe { OwnedHandle::from_raw_handle(handle.0) }
    }

    fn close(handle: OwnedHandle) -> io::Result<()> {
        let handle = HANDLE(handle.into_raw_handle());
        // SAFETY: ownership was transferred out of OwnedHandle for this close.
        if let Err(error) = unsafe { CloseHandle(handle) } {
            // Retain the ordinary RAII retry on a failed close; never spawn.
            drop(owned(handle));
            return Err(win_error(error));
        }
        Ok(())
    }

    fn duplicate(handle: HANDLE, access: u32, inheritable: bool) -> io::Result<OwnedHandle> {
        let mut copy = HANDLE::default();
        // SAFETY: source/target are this process; the returned reference is new.
        unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                handle,
                GetCurrentProcess(),
                &mut copy,
                access,
                inheritable,
                DUPLICATE_HANDLE_OPTIONS(0),
            )
        }
        .map_err(win_error)?;
        Ok(owned(copy))
    }

    pub fn clear_git_selectors_for_private_role() {
        for name in gwt_core::process::GIT_ENV_SCRUB_KEYS {
            // SAFETY: called only at the private binary's single-threaded early
            // entry, before receipt/root work or any spawned threads. No value
            // is read and authentication variables are never changed.
            unsafe { std::env::remove_var(name) };
        }
    }

    fn qpc() -> io::Result<(i64, i64)> {
        let (mut now, mut frequency) = (0, 0);
        // SAFETY: both output pointers remain writable for these calls.
        unsafe {
            QueryPerformanceFrequency(&mut frequency).map_err(win_error)?;
            QueryPerformanceCounter(&mut now).map_err(win_error)?;
        }
        Ok((now, frequency))
    }

    impl Clock {
        pub fn capture() -> io::Result<Self> {
            let (d0_ticks, frequency) = qpc()?;
            let clock = Self {
                d0_ticks,
                frequency,
            };
            clock.remaining_work_at(d0_ticks, frequency)?;
            Ok(clock)
        }

        pub fn remaining_work(self) -> io::Result<Duration> {
            let (now, frequency) = qpc()?;
            self.remaining_work_at(now, frequency)
        }

        pub fn remaining_exit(self) -> io::Result<Duration> {
            let (now, frequency) = qpc()?;
            self.remaining_exit_at(now, frequency)
        }

        pub fn resolution_deadline(self) -> io::Result<gwt_github::client::ResolutionDeadline> {
            // Sampling Instant first prevents projection from adding QPC call time.
            let before = Instant::now();
            let expires_at = before
                .checked_add(self.remaining_work()?)
                .ok_or_else(|| invalid("bootstrap deadline overflow"))?;
            Ok(gwt_github::client::ResolutionDeadline::at(
                expires_at,
                Duration::from_secs(5),
            ))
        }
    }

    pub fn creation_identity(handle: BorrowedHandle<'_>) -> io::Result<CreationIdentity> {
        let (mut created, mut exited, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        // SAFETY: the borrowed exact process remains live; all outputs are valid.
        unsafe {
            GetProcessTimes(
                raw(handle),
                &mut created,
                &mut exited,
                &mut kernel,
                &mut user,
            )
        }
        .map_err(win_error)?;
        let pid = unsafe { GetProcessId(raw(handle)) };
        if pid == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(CreationIdentity {
            pid,
            created: (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime),
        })
    }

    pub fn current_owner_handle() -> io::Result<OwnedHandle> {
        // Duplicate converts the current-process pseudo handle to a real handle.
        duplicate(
            unsafe { GetCurrentProcess() },
            PROCESS_SYNCHRONIZE.0 | PROCESS_QUERY_LIMITED_INFORMATION.0,
            false,
        )
    }

    struct View(MEMORY_MAPPED_VIEW_ADDRESS);
    impl View {
        fn unmap(self) -> io::Result<()> {
            // SAFETY: this object uniquely owns the view. A failure keeps the
            // RAII retry and is returned before any child can be created.
            unsafe { UnmapViewOfFile(self.0) }.map_err(win_error)?;
            std::mem::forget(self);
            Ok(())
        }
    }
    impl Drop for View {
        fn drop(&mut self) {
            // SAFETY: this object uniquely owns the mapped view.
            let _ = unsafe { UnmapViewOfFile(self.0) };
        }
    }

    fn map(
        handle: BorrowedHandle<'_>,
        access: windows::Win32::System::Memory::FILE_MAP,
    ) -> io::Result<View> {
        // SAFETY: the live section is mapped at its fixed bounded length.
        let view = unsafe { MapViewOfFile(raw(handle), access, 0, 0, MAX_BOOTSTRAP_BYTES) };
        if view.Value.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(View(view))
    }

    fn section(descriptor: &Bootstrap) -> io::Result<OwnedHandle> {
        let bytes = descriptor.encode()?;
        // SAFETY: no name means no named-object discovery; fixed paging-file section.
        let writer = owned(
            unsafe {
                CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    None,
                    PAGE_READWRITE,
                    0,
                    MAX_BOOTSTRAP_BYTES as u32,
                    PCWSTR::null(),
                )
            }
            .map_err(win_error)?,
        );
        {
            let view = map(writer.as_handle(), FILE_MAP_WRITE)?;
            // SAFETY: the writable view covers 4096 bytes and lengths are bounded.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    (bytes.len() as u32).to_le_bytes().as_ptr(),
                    view.0.Value.cast::<u8>(),
                    4,
                );
                std::ptr::copy_nonoverlapping(
                    bytes.as_ptr(),
                    view.0.Value.cast::<u8>().add(4),
                    bytes.len(),
                );
            }
            view.unmap()?;
        }
        let reader = duplicate(raw(writer.as_handle()), FILE_MAP_READ.0, true)?;
        // The last writable handle and writable view are gone before spawn.
        close(writer)?;
        Ok(reader)
    }

    fn inherit_argument(value: &str) -> io::Result<OwnedHandle> {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(invalid("invalid bootstrap handle"));
        }
        let number = value
            .parse::<usize>()
            .map_err(|_| invalid("invalid bootstrap handle"))?;
        if number == 0 || number >= usize::MAX - 15 {
            return Err(invalid("invalid bootstrap handle"));
        }
        let handle = HANDLE(number as *mut std::ffi::c_void);
        let mut flags = 0;
        // SAFETY: GetHandleInformation validates this process-local handle value.
        unsafe { GetHandleInformation(handle, &mut flags) }.map_err(win_error)?;
        if flags & HANDLE_FLAG_INHERIT.0 == 0 {
            return Err(invalid("bootstrap handle was not inherited"));
        }
        // SAFETY: after inheritance the receiver owns this handle reference.
        let handle = owned(handle);
        unsafe {
            SetHandleInformation(
                raw(handle.as_handle()),
                HANDLE_FLAG_INHERIT.0,
                HANDLE_FLAGS(0),
            )
        }
        .map_err(win_error)?;
        Ok(handle)
    }

    pub struct ReceivedBootstrap {
        pub descriptor: Bootstrap,
        ack: OwnedHandle,
        owner: Option<OwnedHandle>,
    }

    impl ReceivedBootstrap {
        pub fn acknowledge(&self) -> io::Result<()> {
            // SAFETY: the receiver owns the signal-only event handle.
            unsafe { SetEvent(raw(self.ack.as_handle())) }.map_err(win_error)
        }

        pub fn owner_alive(&self) -> io::Result<bool> {
            match self.owner.as_ref() {
                None => Ok(true),
                Some(owner) => {
                    if Some(creation_identity(owner.as_handle())?) != self.descriptor.owner_identity
                    {
                        return Err(invalid("bootstrap owner identity changed"));
                    }
                    match unsafe { WaitForSingleObject(raw(owner.as_handle()), 0) } {
                        WAIT_TIMEOUT => Ok(true),
                        WAIT_OBJECT_0 => Ok(false),
                        _ => Err(io::Error::last_os_error()),
                    }
                }
            }
        }
    }

    pub fn receive(argv: &[String], role: Role) -> io::Result<ReceivedBootstrap> {
        let expected = if role == Role::Supervisor { 4 } else { 5 };
        if argv.len() != expected
            || argv[1] != role.arg()
            || argv[2..]
                .iter()
                .enumerate()
                .any(|(index, value)| argv[2..2 + index].contains(value))
        {
            return Err(invalid("invalid bootstrap arguments"));
        }
        let section = inherit_argument(&argv[2])?;
        let descriptor = {
            let view = map(section.as_handle(), FILE_MAP_READ)?;
            // SAFETY: the read-only view has the fixed bounded section length.
            let bytes = unsafe {
                std::slice::from_raw_parts(view.0.Value.cast::<u8>(), MAX_BOOTSTRAP_BYTES)
            };
            let length = u32::from_le_bytes(bytes[..4].try_into().expect("fixed header")) as usize;
            if length == 0 || length > MAX_BOOTSTRAP_BYTES - 4 {
                return Err(invalid("invalid bootstrap frame length"));
            }
            Bootstrap::decode(&bytes[4..4 + length], role)?
        };
        drop(section);
        descriptor.clock.remaining_work()?;
        let ack = inherit_argument(&argv[3])?;
        // Reset validates the actual event type without signaling acceptance.
        unsafe { ResetEvent(raw(ack.as_handle())) }.map_err(win_error)?;
        let owner = if role == Role::Sender {
            Some(inherit_argument(&argv[4])?)
        } else {
            None
        };
        let received = ReceivedBootstrap {
            descriptor,
            ack,
            owner,
        };
        if !received.owner_alive()? {
            return Err(invalid("bootstrap owner exited"));
        }
        Ok(received)
    }

    pub struct RawChild {
        process: OwnedHandle,
        primary_thread: Option<OwnedHandle>,
        identity: CreationIdentity,
    }

    impl Drop for RawChild {
        fn drop(&mut self) {
            if self.primary_thread.is_some() {
                // A child never handed to execution must not remain suspended
                // after assignment/startup failure. This is only a stop request.
                let _ = self.request_termination();
            }
        }
    }

    impl RawChild {
        pub fn process_handle(&self) -> BorrowedHandle<'_> {
            self.process.as_handle()
        }
        pub fn resume(&mut self) -> io::Result<()> {
            if creation_identity(self.process.as_handle())? != self.identity {
                return Err(invalid("child identity changed"));
            }
            let thread = self
                .primary_thread
                .as_ref()
                .ok_or_else(|| invalid("child already resumed"))?;
            // SAFETY: this is the retained primary thread returned by CreateProcessW.
            let previous = unsafe { ResumeThread(raw(thread.as_handle())) };
            if previous != 1 {
                return Err(invalid("child did not resume exactly once"));
            }
            self.primary_thread = None;
            Ok(())
        }

        pub fn try_wait(&mut self) -> io::Result<Option<u32>> {
            match unsafe { WaitForSingleObject(raw(self.process.as_handle()), 0) } {
                WAIT_TIMEOUT => Ok(None),
                WAIT_OBJECT_0 => {
                    let mut code = 0;
                    unsafe { GetExitCodeProcess(raw(self.process.as_handle()), &mut code) }
                        .map_err(win_error)?;
                    Ok(Some(code))
                }
                _ => Err(io::Error::last_os_error()),
            }
        }

        /// Requests a stop; success does not prove process or I/O completion.
        pub fn request_termination(&mut self) -> io::Result<()> {
            unsafe { TerminateProcess(raw(self.process.as_handle()), 1) }.map_err(win_error)
        }
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum AckObservation {
        Accepted,
        Exited,
        Unobserved,
    }

    pub struct SpawnedPrivate {
        pub child: RawChild,
        ack: OwnedHandle,
        clock: Clock,
    }

    impl SpawnedPrivate {
        pub fn wait_for_ack(&self) -> io::Result<AckObservation> {
            loop {
                let (now, frequency) = qpc()?;
                let remaining = match self.clock.remaining_at(now, frequency, 2) {
                    Ok(remaining) => remaining,
                    Err(_) => return Ok(AckObservation::Unobserved),
                };
                let millis = remaining.as_millis().clamp(1, 100) as u32;
                let handles = [raw(self.ack.as_handle()), raw(self.child.process_handle())];
                // All borrowed handles stay live until this finite wait returns.
                let observation = match unsafe { WaitForMultipleObjects(&handles, false, millis) } {
                    WAIT_OBJECT_0 => Some(AckObservation::Accepted),
                    event if event.0 == WAIT_OBJECT_0.0 + 1 => Some(AckObservation::Exited),
                    WAIT_TIMEOUT => None,
                    _ => return Err(io::Error::last_os_error()),
                };
                // Native wait timeouts exclude standby on modern Windows;
                // the absolute QPC budget is authoritative after every wake.
                let (now, frequency) = qpc()?;
                if self.clock.remaining_at(now, frequency, 2).is_err() {
                    return Ok(AckObservation::Unobserved);
                }
                if let Some(observation) = observation {
                    return Ok(observation);
                }
            }
        }
    }

    struct Attributes<'a> {
        // Word storage provides native pointer alignment for the opaque list.
        storage: Vec<usize>,
        initialized: bool,
        _handles: &'a [HANDLE],
    }
    impl<'a> Attributes<'a> {
        fn new(handles: &'a [HANDLE]) -> io::Result<Self> {
            let mut bytes = 0;
            // The first call reports the required opaque-list allocation size.
            let _ = unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut bytes) };
            if bytes == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut list = Self {
                storage: vec![0; bytes.div_ceil(std::mem::size_of::<usize>())],
                initialized: false,
                _handles: handles,
            };
            unsafe { InitializeProcThreadAttributeList(Some(list.pointer()), 1, None, &mut bytes) }
                .map_err(win_error)?;
            list.initialized = true;
            unsafe {
                UpdateProcThreadAttribute(
                    list.pointer(),
                    0,
                    PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                    Some(handles.as_ptr().cast()),
                    std::mem::size_of_val(handles),
                    None,
                    None,
                )
            }
            .map_err(win_error)?;
            Ok(list)
        }
        fn pointer(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
            LPPROC_THREAD_ATTRIBUTE_LIST(self.storage.as_mut_ptr().cast())
        }
    }
    impl Drop for Attributes<'_> {
        fn drop(&mut self) {
            if self.initialized {
                unsafe { DeleteProcThreadAttributeList(self.pointer()) };
            }
        }
    }

    fn wide(value: &OsStr) -> io::Result<Vec<u16>> {
        let mut result: Vec<_> = value.encode_wide().collect();
        if result.contains(&0) {
            return Err(invalid("invalid private process path"));
        }
        result.push(0);
        Ok(result)
    }

    pub fn spawn_private(
        root: &Path,
        descriptor: &Bootstrap,
        owner: Option<BorrowedHandle<'_>>,
    ) -> io::Result<SpawnedPrivate> {
        descriptor.clock.remaining_work()?;
        if (descriptor.role == Role::Sender) != owner.is_some()
            || owner.map(creation_identity).transpose()? != descriptor.owner_identity
        {
            return Err(invalid("private owner binding mismatch"));
        }
        let section = section(descriptor)?;
        let event =
            owned(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.map_err(win_error)?);
        let ack = duplicate(raw(event.as_handle()), SYNCHRONIZATION_SYNCHRONIZE.0, false)?;
        let signal = duplicate(raw(event.as_handle()), EVENT_MODIFY_STATE.0, true)?;
        drop(event);
        let owner = owner
            .map(|owner| {
                duplicate(
                    raw(owner),
                    PROCESS_SYNCHRONIZE.0 | PROCESS_QUERY_LIMITED_INFORMATION.0,
                    true,
                )
            })
            .transpose()?;
        let nul_name = wide(OsStr::new("NUL"))?;
        let nul = owned(
            unsafe {
                CreateFileW(
                    PCWSTR(nul_name.as_ptr()),
                    GENERIC_READ.0 | GENERIC_WRITE.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
            }
            .map_err(win_error)?,
        );
        let nul = duplicate(raw(nul.as_handle()), GENERIC_READ.0 | GENERIC_WRITE.0, true)?;
        let mut handles = vec![
            raw(section.as_handle()),
            raw(signal.as_handle()),
            raw(nul.as_handle()),
        ];
        if let Some(owner) = owner.as_ref() {
            handles.push(raw(owner.as_handle()));
        }
        let mut attributes = Attributes::new(&handles)?;
        let exe_path = std::env::current_exe()?;
        let exe = wide(exe_path.as_os_str())?;
        // Only fixed role and decimal handle arguments follow the trusted argv0.
        let mut command: Vec<u16> = OsStr::new("\"")
            .encode_wide()
            .chain(exe_path.as_os_str().encode_wide())
            .chain(OsStr::new("\" ").encode_wide())
            .collect();
        let arguments = format!(
            "{} {} {}{}",
            descriptor.role.arg(),
            handles[0].0 as usize,
            handles[1].0 as usize,
            owner
                .as_ref()
                .map(|owner| format!(" {}", raw(owner.as_handle()).0 as usize))
                .unwrap_or_default()
        );
        command.extend(OsStr::new(&arguments).encode_wide());
        command.push(0);
        let cwd = wide(gwt_core::paths::normalize_windows_child_process_path(root).as_os_str())?;
        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.StartupInfo.hStdInput = raw(nul.as_handle());
        startup.StartupInfo.hStdOutput = raw(nul.as_handle());
        startup.StartupInfo.hStdError = raw(nul.as_handle());
        startup.lpAttributeList = attributes.pointer();
        let mut info = PROCESS_INFORMATION::default();
        descriptor.clock.remaining_work()?;
        // NULL environment preserves ordinary inheritance, including auth origin.
        // No breakaway/console flags or arbitrary process configuration are accepted.
        unsafe {
            CreateProcessW(
                PCWSTR(exe.as_ptr()),
                Some(PWSTR(command.as_mut_ptr())),
                None,
                None,
                true,
                CREATE_NO_WINDOW | CREATE_SUSPENDED | EXTENDED_STARTUPINFO_PRESENT,
                None,
                PCWSTR(cwd.as_ptr()),
                &startup.StartupInfo,
                &mut info,
            )
        }
        .map_err(win_error)?;
        let process = owned(info.hProcess);
        let thread = owned(info.hThread);
        let identity = match creation_identity(process.as_handle()) {
            Ok(identity) if identity.pid == info.dwProcessId => identity,
            _ => {
                let _ = unsafe { TerminateProcess(raw(process.as_handle()), 1) };
                return Err(invalid("private child identity unavailable"));
            }
        };
        let mut child = RawChild {
            process,
            primary_thread: Some(thread),
            identity,
        };
        if descriptor.role == Role::Supervisor {
            if let Err(error) = child.resume() {
                let _ = child.request_termination();
                return Err(error);
            }
        }
        Ok(SpawnedPrivate {
            child,
            ack,
            clock: descriptor.clock,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::Foundation::WAIT_FAILED;

        fn descriptor() -> Bootstrap {
            Bootstrap::supervisor(
                Uuid::from_u128(1),
                Uuid::from_u128(2),
                Uuid::from_u128(3),
                Clock::capture().unwrap(),
            )
        }

        #[test]
        fn inherited_section_allows_reading_and_denies_writable_views() {
            let frame = descriptor();
            let reader = section(&frame).unwrap();
            assert!(map(reader.as_handle(), FILE_MAP_WRITE).is_err());
            let view = map(reader.as_handle(), FILE_MAP_READ).unwrap();
            let bytes = unsafe {
                std::slice::from_raw_parts(view.0.Value.cast::<u8>(), MAX_BOOTSTRAP_BYTES)
            };
            let length = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
            assert_eq!(
                Bootstrap::decode(&bytes[4..4 + length], Role::Supervisor).unwrap(),
                frame
            );
        }

        #[test]
        fn ack_handles_separate_signal_from_wait_permission() {
            let event = owned(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.unwrap());
            let signal = duplicate(raw(event.as_handle()), EVENT_MODIFY_STATE.0, true).unwrap();
            let wait =
                duplicate(raw(event.as_handle()), SYNCHRONIZATION_SYNCHRONIZE.0, false).unwrap();
            drop(event);
            assert!(unsafe { SetEvent(raw(wait.as_handle())) }.is_err());
            assert_eq!(
                unsafe { WaitForSingleObject(raw(signal.as_handle()), 0) },
                WAIT_FAILED
            );
            unsafe { SetEvent(raw(signal.as_handle())) }.unwrap();
            assert_eq!(
                unsafe { WaitForSingleObject(raw(wait.as_handle()), 0) },
                WAIT_OBJECT_0
            );
        }

        #[test]
        fn owner_liveness_rejects_a_different_creation_identity() {
            let owner = current_owner_handle().unwrap();
            let mut identity = creation_identity(owner.as_handle()).unwrap();
            let event = owned(unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.unwrap());
            let mut received = ReceivedBootstrap {
                descriptor: descriptor().sender(identity),
                ack: event,
                owner: Some(owner),
            };
            assert!(received.owner_alive().unwrap());
            identity.created += 1;
            received.descriptor.owner_identity = Some(identity);
            assert!(received.owner_alive().is_err());
            assert!(inherit_argument("0").is_err());
            assert!(inherit_argument("-1").is_err());
            assert!(inherit_argument(&usize::MAX.to_string()).is_err());
        }

        #[test]
        fn receiver_rejects_a_valid_section_with_a_non_event_ack() {
            let frame = descriptor();
            let section = section(&frame).unwrap();
            // Bit 2 is a valid process right as well as EVENT_MODIFY_STATE.
            // This is a live inherited kernel handle, but not an event object.
            let wrong_ack =
                duplicate(unsafe { GetCurrentProcess() }, EVENT_MODIFY_STATE.0, true).unwrap();
            let argv = vec![
                "gwtd".to_string(),
                Role::Supervisor.arg().to_string(),
                (section.into_raw_handle() as usize).to_string(),
                (wrong_ack.into_raw_handle() as usize).to_string(),
            ];
            // Ownership transfers once through argv. receive closes both
            // references on rejection; the test must not close them again.
            assert!(receive(&argv, Role::Supervisor).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor() -> Bootstrap {
        Bootstrap::supervisor(
            Uuid::from_u128(1),
            Uuid::from_u128(2),
            Uuid::from_u128(3),
            Clock {
                d0_ticks: 10_000,
                frequency: 1_000,
            },
        )
    }

    #[test]
    fn bootstrap_accepts_closed_supervisor_frame() {
        let frame = descriptor();
        let encoded = frame.encode().expect("encode fixed frame");
        assert_eq!(
            Bootstrap::decode(&encoded, Role::Supervisor).unwrap(),
            frame
        );
        let sender = frame.sender(CreationIdentity { pid: 1, created: 2 });
        assert_eq!(
            Bootstrap::decode(&sender.encode().unwrap(), Role::Sender).unwrap(),
            sender
        );
    }

    #[test]
    fn bootstrap_rejects_wrong_binding_unknown_fields_and_oversized_frames() {
        let frame = descriptor();
        let encoded = serde_json::to_vec(&frame).unwrap();
        assert!(Bootstrap::decode(&encoded, Role::Sender).is_err());
        let mut value = serde_json::to_value(&frame).unwrap();
        value["token"] = serde_json::json!("dummy");
        assert!(Bootstrap::decode(&serde_json::to_vec(&value).unwrap(), Role::Supervisor).is_err());
        let mut missing = serde_json::to_value(&frame).unwrap();
        missing.as_object_mut().unwrap().remove("owner_identity");
        assert!(
            Bootstrap::decode(&serde_json::to_vec(&missing).unwrap(), Role::Supervisor).is_err()
        );
        assert!(Bootstrap::decode(&vec![b' '; MAX_BOOTSTRAP_BYTES + 1], Role::Supervisor).is_err());
        let mut frame = frame;
        frame.owner_identity = Some(CreationIdentity { pid: 1, created: 2 });
        assert!(Bootstrap::decode(&serde_json::to_vec(&frame).unwrap(), Role::Supervisor).is_err());
    }

    #[test]
    fn clock_preserves_d0_across_work_and_exit_budgets() {
        let clock = descriptor().clock;
        assert_eq!(
            clock.remaining_work_at(25_000, 1_000).unwrap(),
            Duration::from_millis(99_999)
        );
        assert_eq!(
            clock.remaining_exit_at(25_000, 1_000).unwrap(),
            Duration::from_millis(104_999)
        );
        assert!(clock.remaining_work_at(125_000, 1_000).is_err());
        assert!(clock.remaining_work_at(25_000, 2_000).is_err());
        assert!(clock.remaining_work_at(9_000, 1_000).is_err());
        assert!(Clock {
            d0_ticks: i64::MAX,
            frequency: 1_000
        }
        .remaining_work_at(i64::MAX, 1_000)
        .is_err());
    }

    #[test]
    fn receiver_validates_the_ack_event_before_returning_admission() {
        let source = include_str!("feedback_owner_bootstrap.rs");
        let receive = source
            .split("\n    pub fn receive(")
            .nth(1)
            .and_then(|tail| tail.split("\n    pub struct RawChild").next())
            .expect("private receive entry");
        let validate = receive
            .find("ResetEvent(raw(ack.as_handle()))")
            .expect("validate actual event kind and modify-state access");
        let admission = receive
            .find("let received = ReceivedBootstrap")
            .expect("construct received admission");
        assert!(validate < admission);
    }
}
