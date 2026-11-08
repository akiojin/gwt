//! One-shot native input acceptance shared with the physical owner process.
//!
//! Revocation and physical acceptance race on the same process-shared atomic.
//! An accepted effect may finish after revocation; it cannot be accepted again.

use serde::{Deserialize, Serialize};
use std::{fmt, io, path::PathBuf, sync::Arc};

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeInputPermitRef {
    path: PathBuf,
    nonce: String,
    device: u64,
    inode: u64,
    host_pid: u32,
    host_birth: u64,
}

impl fmt::Debug for NativeInputPermitRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeInputPermitRef")
            .finish_non_exhaustive()
    }
}

#[cfg(all(unix, target_has_atomic = "32", target_has_atomic = "64"))]
mod platform {
    use super::*;
    use std::{
        fs::{File, OpenOptions},
        io::{Read, Write},
        os::{
            fd::AsRawFd,
            unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
        },
        path::Path,
        ptr::NonNull,
        sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        time::{Instant, SystemTime, UNIX_EPOCH},
    };
    use tokio::sync::Notify;

    const FILE_SIZE: usize = 4096;
    const HEADER_SIZE: usize = 512;
    const STATE: usize = 512;
    const ISSUED: usize = 520;
    const EXPIRES: usize = 528;
    const DEADLINE: usize = 536;
    const UNARMED: u32 = 0;
    const ARMED: u32 = 1;
    const ACCEPTED: u32 = 2;
    const REVOKED: u32 = 3;

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Header {
        version: u32,
        nonce: String,
        host_pid: u32,
        host_birth: u64,
    }

    fn refused() -> io::Error {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "native input permit is unavailable",
        )
    }

    fn monotonic_ns() -> io::Result<u64> {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `time` is writable for the syscall and CLOCK_MONOTONIC is a
        // system-wide clock shared by the Host and owner on this machine.
        if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) } != 0 {
            return Err(io::Error::last_os_error());
        }
        u64::try_from(time.tv_sec)
            .ok()
            .and_then(|seconds| seconds.checked_mul(1_000_000_000))
            .and_then(|value| value.checked_add(u64::try_from(time.tv_nsec).ok()?))
            .ok_or_else(refused)
    }

    struct Mapping {
        pointer: NonNull<u8>,
        _file: File,
    }
    // SAFETY: after publication the fixed-size mapping is accessed only through
    // aligned lock-free atomic fields. Its owner never truncates the file, and
    // unmapping happens only after all references to this Mapping are dropped.
    unsafe impl Send for Mapping {}
    // SAFETY: the same atomic-only and lifetime guarantees permit shared reads.
    unsafe impl Sync for Mapping {}

    impl Mapping {
        fn new(file: File) -> io::Result<Self> {
            // SAFETY: the file was checked/created as a private regular 4KiB
            // file. Keeping its descriptor and mapping alive prevents unlink
            // from invalidating memory. No supported operation truncates it.
            let pointer = unsafe {
                libc::mmap(
                    std::ptr::null_mut(),
                    FILE_SIZE,
                    libc::PROT_READ | libc::PROT_WRITE,
                    libc::MAP_SHARED,
                    file.as_raw_fd(),
                    0,
                )
            };
            if pointer == libc::MAP_FAILED {
                return Err(io::Error::last_os_error());
            }
            let pointer = NonNull::new(pointer.cast()).ok_or_else(refused)?;
            Ok(Self {
                pointer,
                _file: file,
            })
        }

        fn state(&self) -> &AtomicU32 {
            // SAFETY: mmap is page-aligned, offset 512 is AtomicU32-aligned,
            // initialized zero bytes form a valid value, and this field is
            // never accessed non-atomically after publication.
            unsafe { AtomicU32::from_ptr(self.pointer.as_ptr().add(STATE).cast()) }
        }

        fn time(&self, offset: usize) -> &AtomicU64 {
            debug_assert!([ISSUED, EXPIRES, DEADLINE].contains(&offset));
            // SAFETY: all callers use the three fixed 8-byte-aligned offsets
            // inside the mapping; these fields are accessed only atomically.
            unsafe { AtomicU64::from_ptr(self.pointer.as_ptr().add(offset).cast()) }
        }
    }

    impl Drop for Mapping {
        fn drop(&mut self) {
            // SAFETY: this is the original mmap address/length, and Rust's
            // borrowing/Arc lifetime guarantees no atomic reference survives.
            let _ = unsafe { libc::munmap(self.pointer.as_ptr().cast(), FILE_SIZE) };
        }
    }

    pub struct NativeInputPermit {
        mapping: Mapping,
        reference: NativeInputPermitRef,
        _directory: tempfile::TempDir,
        arm_started: AtomicBool,
        notify: Notify,
    }

    impl NativeInputPermit {
        pub fn new() -> io::Result<Arc<Self>> {
            let directory = tempfile::Builder::new()
                .prefix(".gwt-mobile-input-")
                .permissions(std::fs::Permissions::from_mode(0o700))
                .tempdir()?;
            let path = directory.path().join("permit");
            private_parent(&path)?;
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)?;
            // This descriptor is held for the creator's lifetime. Kernel
            // close releases its lease even when a dead Host remains a zombie.
            // PC revoke itself never acquires or waits for this file lock.
            fs2::FileExt::lock_exclusive(&file)?;
            file.set_len(FILE_SIZE as u64)?;
            let host_pid = std::process::id();
            let host_birth =
                crate::process::host_process_start_time(host_pid).ok_or_else(refused)?;
            let nonce = uuid::Uuid::new_v4().to_string();
            let header = serde_json::to_vec(&Header {
                version: 1,
                nonce: nonce.clone(),
                host_pid,
                host_birth,
            })
            .map_err(io::Error::other)?;
            if header.len() > HEADER_SIZE - 4 {
                return Err(refused());
            }
            file.write_all(&(header.len() as u32).to_le_bytes())?;
            file.write_all(&header)?;
            let metadata = file.metadata()?;
            let reference = NativeInputPermitRef {
                path,
                nonce,
                device: metadata.dev(),
                inode: metadata.ino(),
                host_pid,
                host_birth,
            };
            let mapping = Mapping::new(file)?;
            mapping.state().store(UNARMED, Ordering::Release);
            Ok(Arc::new(Self {
                mapping,
                reference,
                _directory: directory,
                arm_started: AtomicBool::new(false),
                notify: Notify::new(),
            }))
        }

        pub fn reference(&self) -> NativeInputPermitRef {
            self.reference.clone()
        }

        #[cfg(test)]
        pub(crate) fn release_creator_lease_for_test(&self) {
            fs2::FileExt::unlock(&self.mapping._file).unwrap();
        }

        pub fn arm(&self, issued_at: u64, expires_at: u64, now: u64) -> io::Result<()> {
            self.arm_until(issued_at, expires_at, now, None)
        }

        /// Cap acceptance at the absolute registration deadline as well as the
        /// device/session expiry. Sampling the raw clock first makes conversion
        /// conservative even if this thread pauses before sampling Instant.
        pub fn arm_until(
            &self,
            issued_at: u64,
            expires_at: u64,
            now: u64,
            registration_deadline: Option<Instant>,
        ) -> io::Result<()> {
            if now < issued_at || now >= expires_at {
                return Err(refused());
            }
            let raw_before = monotonic_ns()?;
            let mut deadline = (expires_at - now)
                .checked_mul(1_000_000_000)
                .and_then(|remaining| raw_before.checked_add(remaining))
                .ok_or_else(refused)?;
            if let Some(registration_deadline) = registration_deadline {
                let remaining = registration_deadline
                    .checked_duration_since(Instant::now())
                    .filter(|remaining| !remaining.is_zero())
                    .ok_or_else(refused)?;
                let remaining = u64::try_from(remaining.as_nanos()).map_err(|_| refused())?;
                deadline = deadline.min(raw_before.checked_add(remaining).ok_or_else(refused)?);
            }
            // Exactly one caller may publish deadline fields; a failed re-arm
            // must never overwrite the bounds seen by the native process.
            self.arm_started
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| refused())?;
            self.mapping
                .time(ISSUED)
                .store(issued_at, Ordering::Relaxed);
            self.mapping
                .time(EXPIRES)
                .store(expires_at, Ordering::Relaxed);
            self.mapping
                .time(DEADLINE)
                .store(deadline, Ordering::Relaxed);
            self.mapping
                .state()
                .compare_exchange(UNARMED, ARMED, Ordering::Release, Ordering::Acquire)
                .map(|_| ())
                .map_err(|_| refused())
        }

        pub fn revoke(&self) {
            self.mapping.state().swap(REVOKED, Ordering::AcqRel);
            self.notify.notify_waiters();
        }

        pub fn is_revoked(&self) -> bool {
            self.mapping.state().load(Ordering::Acquire) == REVOKED
        }

        pub async fn revoked(&self) {
            loop {
                let changed = self.notify.notified();
                tokio::pin!(changed);
                changed.as_mut().enable();
                if self.is_revoked() {
                    return;
                }
                changed.await;
            }
        }
    }

    impl Drop for NativeInputPermit {
        fn drop(&mut self) {
            self.revoke();
        }
    }

    pub struct OwnerInputPermit {
        mapping: Mapping,
        reference: NativeInputPermitRef,
    }

    fn private_parent(path: &Path) -> io::Result<()> {
        let parent = std::fs::symlink_metadata(path.parent().ok_or_else(refused)?)?;
        // SAFETY: geteuid has no pointer arguments or additional preconditions.
        if !parent.is_dir()
            || parent.mode() & 0o777 != 0o700
            || parent.uid() != unsafe { libc::geteuid() }
        {
            return Err(refused());
        }
        Ok(())
    }

    impl NativeInputPermitRef {
        pub fn open(&self) -> io::Result<OwnerInputPermit> {
            private_parent(&self.path)?;
            let mut file = OpenOptions::new()
                .read(true)
                .write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                .open(&self.path)?;
            let metadata = file.metadata()?;
            // SAFETY: geteuid has no pointer arguments or additional preconditions.
            if !metadata.is_file()
                || metadata.len() != FILE_SIZE as u64
                || metadata.mode() & 0o777 != 0o600
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.dev() != self.device
                || metadata.ino() != self.inode
                || metadata.nlink() != 1
            {
                return Err(refused());
            }
            let mut bytes = [0; HEADER_SIZE];
            file.read_exact(&mut bytes)?;
            let length = u32::from_le_bytes(bytes[..4].try_into().expect("four-byte header length"))
                as usize;
            if length > HEADER_SIZE - 4 {
                return Err(refused());
            }
            let header: Header =
                serde_json::from_slice(&bytes[4..4 + length]).map_err(|_| refused())?;
            if header.version != 1
                || header.nonce != self.nonce
                || header.host_pid != self.host_pid
                || header.host_birth != self.host_birth
                || crate::process::host_process_start_time(self.host_pid) != Some(self.host_birth)
            {
                return Err(refused());
            }
            Ok(OwnerInputPermit {
                mapping: Mapping::new(file)?,
                reference: self.clone(),
            })
        }
    }

    impl OwnerInputPermit {
        pub fn is_revoked(&self) -> bool {
            self.mapping.state().load(Ordering::Acquire) == REVOKED || self.creator_released()
        }

        /// Positive proof that the creator's descriptor lease disappeared.
        /// Unknown I/O keeps receipts rather than reclaiming an active caller.
        pub(crate) fn creator_released(&self) -> bool {
            match fs2::FileExt::try_lock_shared(&self.mapping._file) {
                Ok(()) => fs2::FileExt::unlock(&self.mapping._file).is_ok(),
                Err(_) => false,
            }
        }

        fn check_time_and_host(
            &self,
            clock: &impl Fn() -> io::Result<(u64, u64)>,
        ) -> io::Result<()> {
            // A matching PID/birth may still describe a zombie. Only a live
            // creator descriptor holds this exclusive lease. The owner's
            // independent open uses a nonblocking query and holds no lease.
            match fs2::FileExt::try_lock_shared(&self.mapping._file) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Ok(()) => {
                    let _ = fs2::FileExt::unlock(&self.mapping._file);
                    return Err(refused());
                }
                Err(_) => return Err(refused()),
            }
            let (now, monotonic) = clock()?;
            if now < self.mapping.time(ISSUED).load(Ordering::Relaxed)
                || now >= self.mapping.time(EXPIRES).load(Ordering::Relaxed)
                || monotonic >= self.mapping.time(DEADLINE).load(Ordering::Relaxed)
                || crate::process::host_process_start_time(self.reference.host_pid)
                    != Some(self.reference.host_birth)
            {
                return Err(refused());
            }
            Ok(())
        }

        /// Call only with the physical writer held and all owner identity,
        /// pane/Session and payload checks satisfied. This CAS is acceptance.
        pub fn accept(&self) -> io::Result<()> {
            self.accept_with_clock(&|| {
                Ok((
                    SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map_err(|_| refused())?
                        .as_secs(),
                    monotonic_ns()?,
                ))
            })
        }

        fn accept_with_clock(&self, clock: &impl Fn() -> io::Result<(u64, u64)>) -> io::Result<()> {
            if self.mapping.state().load(Ordering::Acquire) != ARMED {
                return Err(refused());
            }
            self.check_time_and_host(clock)?;
            self.mapping
                .state()
                .compare_exchange(ARMED, ACCEPTED, Ordering::AcqRel, Ordering::Acquire)
                .map_err(|_| refused())?;
            // A late expiry or observed creator death cannot authorize a write.
            // Acceptance remains consumed; callers never retry this operation.
            self.check_time_and_host(clock)
        }
    }

    #[cfg(test)]
    mod registration_deadline_tests {
        use super::*;
        use std::time::{Duration, Instant};

        #[test]
        fn registration_deadline_caps_the_actual_owner_acceptance() {
            let wall = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs();
            let permit = NativeInputPermit::new().unwrap();
            let owner = permit.reference().open().unwrap();
            let before = monotonic_ns().unwrap();
            permit
                .arm_until(
                    wall,
                    wall + 300,
                    wall,
                    // test-hygiene: allow-wall-clock-deadline Exercises production Instant-to-kernel-clock conversion; expiry is checked with the injected owner clock, never by sleeping.
                    Some(Instant::now() + Duration::from_secs(30)),
                )
                .unwrap();
            let cap = owner.mapping.time(DEADLINE).load(Ordering::Relaxed);
            assert!(cap > before);
            assert!(cap <= monotonic_ns().unwrap() + 30_000_000_000);
            assert!(
                owner.accept_with_clock(&|| Ok((wall, cap))).is_err(),
                "lease boundary must reject without a Host recheck"
            );
            assert_eq!(owner.mapping.state().load(Ordering::Acquire), ARMED);
            owner.accept().unwrap();
            assert!(owner.accept().is_err());
            let elapsed = NativeInputPermit::new().unwrap();
            assert!(elapsed
                .arm_until(
                    wall,
                    wall + 300,
                    wall,
                    // test-hygiene: allow-wall-clock-deadline A past Instant deterministically rejects arming; no real-time deadline is awaited.
                    Some(Instant::now() - Duration::from_secs(1))
                )
                .is_err());
            assert!(elapsed.reference().open().unwrap().accept().is_err());
        }
    }
}

#[cfg(all(unix, target_has_atomic = "32", target_has_atomic = "64"))]
pub use platform::{NativeInputPermit, OwnerInputPermit};

#[cfg(not(all(unix, target_has_atomic = "32", target_has_atomic = "64")))]
mod platform {
    use super::*;
    fn unsupported() -> io::Error {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "native mobile permits require Unix lock-free atomics",
        )
    }
    pub struct NativeInputPermit;
    pub struct OwnerInputPermit;
    impl NativeInputPermit {
        pub fn new() -> io::Result<Arc<Self>> {
            Err(unsupported())
        }
        pub fn reference(&self) -> NativeInputPermitRef {
            unreachable!("unsupported permits cannot be created")
        }
        pub fn arm_until(
            &self,
            _: u64,
            _: u64,
            _: u64,
            _: Option<std::time::Instant>,
        ) -> io::Result<()> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "native mobile input requires Unix shared atomics",
            ))
        }

        pub fn arm(&self, _: u64, _: u64, _: u64) -> io::Result<()> {
            Err(unsupported())
        }
        pub fn revoke(&self) {}
        pub fn is_revoked(&self) -> bool {
            true
        }
        pub async fn revoked(&self) {}
    }
    impl NativeInputPermitRef {
        pub fn open(&self) -> io::Result<OwnerInputPermit> {
            Err(unsupported())
        }
    }
    impl OwnerInputPermit {
        pub fn is_revoked(&self) -> bool {
            true
        }
        pub(crate) fn creator_released(&self) -> bool {
            false
        }
        pub fn accept(&self) -> io::Result<()> {
            Err(unsupported())
        }
    }
}
#[cfg(not(all(unix, target_has_atomic = "32", target_has_atomic = "64")))]
pub use platform::{NativeInputPermit, OwnerInputPermit};

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    #[test]
    fn private_permit_is_unarmed_one_shot_and_revocable_after_mapping() {
        let permit = NativeInputPermit::new().unwrap();
        let reference = permit.reference();
        assert_eq!(
            std::fs::metadata(reference.path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&reference.path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let owner = permit.reference().open().unwrap();
        assert!(
            owner.accept().is_err(),
            "Prepare must grant no input authority"
        );
        let time = now();
        permit.arm(time, time + 300, time).unwrap();
        owner.accept().unwrap();
        assert!(
            owner.accept().is_err(),
            "one input may be accepted only once"
        );
        permit.revoke();
        assert!(permit.is_revoked());
        assert!(owner.accept().is_err());
        assert!(permit.arm(time, time + 300, time).is_err());

        let revoked = NativeInputPermit::new().unwrap();
        let waiting_owner = revoked.reference().open().unwrap();
        revoked.arm(time, time + 300, time).unwrap();
        revoked.revoke();
        assert!(
            waiting_owner.accept().is_err(),
            "revoke must reach an existing owner mapping"
        );
    }

    #[test]
    fn permit_reference_refuses_substitution_permissions_and_expiry() {
        let permit = NativeInputPermit::new().unwrap();
        let mut foreign = permit.reference();
        foreign.nonce = uuid::Uuid::new_v4().to_string();
        assert!(foreign.open().is_err());
        let reference = permit.reference();
        assert!(!format!("{reference:?}").contains(&reference.nonce));
        assert!(!format!("{reference:?}").contains(&reference.path.display().to_string()));
        std::fs::set_permissions(&reference.path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(reference.open().is_err());
        std::fs::set_permissions(&reference.path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let owner = reference.open().unwrap();
        let time = now();
        assert!(permit.arm(time + 10, time + 20, time).is_err());
        permit.arm(time - 20, time - 10, time - 15).unwrap();
        assert!(
            owner.accept().is_err(),
            "owner must recheck wall-clock expiry"
        );

        let lost_creator = NativeInputPermit::new().unwrap();
        let owner = lost_creator.reference().open().unwrap();
        lost_creator.arm(time, time + 300, time).unwrap();
        lost_creator.release_creator_lease_for_test();
        assert!(
            owner.is_revoked(),
            "creator lease loss must revoke an existing owner mapping"
        );
        assert!(
            owner.accept().is_err(),
            "a matching PID/birth must not replace the creator's live file lease"
        );
    }
}
