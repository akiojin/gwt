//! Thread-scoped absolute deadlines for synchronous local operations.

use std::{
    cell::Cell,
    fs::File,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use fs2::FileExt;

const LOCK_POLL_INTERVAL: Duration = Duration::from_millis(10);

thread_local! {
    static CURRENT_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) };
    #[cfg(any(test, feature = "test-support"))]
    static TEST_NOW: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// Read the operation clock used both to create and to enforce a deadline.
pub fn now() -> Instant {
    #[cfg(any(test, feature = "test-support"))]
    if let Some(now) = TEST_NOW.with(Cell::get) {
        return now;
    }
    Instant::now()
}

/// Fix synchronous operation time while a test exercises a transaction.
/// Keep this guard on its creating thread and outside async suspension points.
#[cfg(any(test, feature = "test-support"))]
pub struct ScopedOperationClock {
    previous: Option<Instant>,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}

#[cfg(any(test, feature = "test-support"))]
impl ScopedOperationClock {
    pub fn set(now: Instant) -> Self {
        Self {
            previous: TEST_NOW.with(|current| current.replace(Some(now))),
            _thread: std::marker::PhantomData,
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for ScopedOperationClock {
    fn drop(&mut self) {
        TEST_NOW.with(|current| current.set(self.previous));
    }
}

#[derive(Debug)]
pub struct ScopedOperationDeadline {
    previous: Option<Instant>,
}

impl ScopedOperationDeadline {
    pub fn enter(deadline: Instant) -> Self {
        let previous = CURRENT_DEADLINE.with(|current| {
            let previous = current.get();
            current.set(Some(previous.map_or(deadline, |value| value.min(deadline))));
            previous
        });
        Self { previous }
    }
}

impl Drop for ScopedOperationDeadline {
    fn drop(&mut self) {
        CURRENT_DEADLINE.with(|current| current.set(self.previous));
    }
}

pub fn current() -> Option<Instant> {
    CURRENT_DEADLINE.with(Cell::get)
}

/// Whether a `try_lock_*` failure means "another holder has the lock".
///
/// Unix reports contention as [`io::ErrorKind::WouldBlock`], while Windows
/// returns `ERROR_LOCK_VIOLATION` (raw OS error 33), which `std` still maps to
/// `Uncategorized`. Comparing against fs2's canonical contention error covers
/// both without hard-coding the platform error number.
pub fn is_lock_contended(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::WouldBlock
        || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
}

/// An exclusive file lock with best-effort observational holder diagnostics.
/// Metadata never grants authority: it may be stale after a crash or unreadable
/// while a holder updates it. The OS lock alone determines ownership.
#[derive(Debug)]
pub struct NamedFileLock {
    file: File,
    holder_path: PathBuf,
    locked: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ObservedLockHolder {
    pid: u32,
    operation: String,
    acquired_at: String,
}

impl NamedFileLock {
    /// Wait for ownership using the ambient operation deadline.
    pub fn acquire(path: &Path, operation: &str) -> io::Result<Self> {
        let file = open_named_lock(path)?;
        let holder_path = named_lock_holder_path(path);
        lock_exclusive_with_observer(&file, || {
            let _ = named_lock_error(
                &holder_path,
                operation,
                io::Error::new(io::ErrorKind::WouldBlock, "file lock contended"),
            );
        })
        .map_err(|error| named_lock_error(&holder_path, operation, error))?;
        Ok(Self::record(file, holder_path, operation))
    }

    /// Try once. Only actual OS lock contention is returned as `WouldBlock`.
    pub fn try_acquire(path: &Path, operation: &str) -> io::Result<Self> {
        let file = open_named_lock(path)?;
        let holder_path = named_lock_holder_path(path);
        file.try_lock_exclusive().map_err(|error| {
            let error = if is_lock_contended(&error) {
                io::Error::new(io::ErrorKind::WouldBlock, error)
            } else {
                error
            };
            named_lock_error(&holder_path, operation, error)
        })?;
        Ok(Self::record(file, holder_path, operation))
    }

    fn record(file: File, holder_path: PathBuf, operation: &str) -> Self {
        let guard = Self {
            file,
            holder_path,
            locked: true,
        };
        let holder = ObservedLockHolder {
            pid: std::process::id(),
            operation: operation.to_owned(),
            acquired_at: chrono::Utc::now().to_rfc3339(),
        };
        let recorded = (|| -> io::Result<()> {
            let bytes = serde_json::to_vec(&holder)?;
            match std::fs::remove_file(&guard.holder_path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            // Do not follow a stale symlink or a node replaced after removal.
            File::options()
                .create_new(true)
                .write(true)
                .open(&guard.holder_path)?
                .write_all(&bytes)
        })();
        if let Err(error) = recorded {
            tracing::warn!(operation, %error, "could not record observational lock holder metadata");
        }
        guard
    }

    /// Clear diagnostic metadata before releasing ownership; keep the inode.
    pub fn unlock(mut self) -> io::Result<()> {
        self.release()
    }

    fn release(&mut self) -> io::Result<()> {
        if !self.locked {
            return Ok(());
        }
        if let Err(error) = std::fs::remove_file(&self.holder_path) {
            if error.kind() != io::ErrorKind::NotFound {
                tracing::warn!(%error, "could not clear observational lock holder metadata");
            }
        }
        let unlocked = FileExt::unlock(&self.file);
        if unlocked.is_ok() {
            self.locked = false;
        }
        unlocked
    }
}

impl Drop for NamedFileLock {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

fn open_named_lock(path: &Path) -> io::Result<File> {
    File::options()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(path)
}

fn named_lock_holder_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(".holder.json");
    PathBuf::from(name)
}

fn open_named_lock_holder(path: &Path) -> io::Result<File> {
    let mut options = File::options();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // A raced-in FIFO must not wait for a writer; a symlink is never read.
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT.0);
    }
    let file = options.open(path)?;
    let file_type = file.metadata()?.file_type();
    if !file_type.is_file() || file_type.is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "lock holder metadata is not a regular file",
        ));
    }
    Ok(file)
}

fn named_lock_error(holder_path: &Path, operation: &str, error: io::Error) -> io::Error {
    // Bound diagnostic IO, and do not require valid metadata from old writers.
    let holder = std::fs::symlink_metadata(holder_path)
        .ok()
        .filter(|metadata| metadata.file_type().is_file())
        .and_then(|_| open_named_lock_holder(holder_path).ok())
        .and_then(|reader| {
            serde_json::from_reader::<_, ObservedLockHolder>(reader.take(4096)).ok()
        });
    let pid = holder
        .as_ref()
        .map(|holder| holder.pid.to_string())
        .unwrap_or_else(|| "unknown".to_owned());
    let holder_operation = holder
        .as_ref()
        .map(|holder| holder.operation.as_str())
        .unwrap_or("unknown");
    let acquired_at = holder
        .as_ref()
        .map(|holder| holder.acquired_at.as_str())
        .unwrap_or("unknown");
    tracing::warn!(
        operation, observed_holder_pid = %pid,
        observed_holder_operation = holder_operation,
        observed_holder_acquired_at = acquired_at, %error,
        "named file lock acquisition failed or contended; holder metadata is observational"
    );
    io::Error::new(error.kind(), format!(
        "{error}; operation={operation}; observed holder pid={pid}, operation={holder_operation}, acquired_at={acquired_at} (metadata may be stale)"
    ))
}

pub fn lock_exclusive(file: &File) -> io::Result<()> {
    lock_exclusive_with_observer(file, || {})
}

/// Acquire an exclusive lock and report the first contention observed by this
/// exact call. The callback is a causal test/instrumentation boundary; lock
/// timing and deadline behavior remain identical to [`lock_exclusive`].
pub fn lock_exclusive_with_observer(
    file: &File,
    mut on_first_contention: impl FnMut(),
) -> io::Result<()> {
    let Some(deadline) = current() else {
        return match file.try_lock_exclusive() {
            Ok(()) => Ok(()),
            Err(error) if is_lock_contended(&error) => {
                on_first_contention();
                FileExt::lock_exclusive(file)
            }
            Err(error) => Err(error),
        };
    };
    let mut contention_reported = false;
    loop {
        if now() >= deadline {
            return Err(deadline_error("file lock"));
        }
        match file.try_lock_exclusive() {
            Ok(()) => {
                if now() >= deadline {
                    FileExt::unlock(file)?;
                    return Err(deadline_error("file lock"));
                }
                return Ok(());
            }
            Err(error) if is_lock_contended(&error) => {
                if !contention_reported {
                    on_first_contention();
                    contention_reported = true;
                }
                let now = now();
                if now >= deadline {
                    return Err(deadline_error("file lock"));
                }
                std::thread::sleep(LOCK_POLL_INTERVAL.min(deadline.saturating_duration_since(now)));
            }
            Err(error) => return Err(error),
        }
    }
}

pub fn ensure_remaining(operation: &str) -> io::Result<Option<Instant>> {
    let deadline = current();
    if deadline.is_some_and(|deadline| now() >= deadline) {
        return Err(deadline_error(operation));
    }
    Ok(deadline)
}

fn deadline_error(operation: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::TimedOut,
        format!("operation deadline expired during {operation}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_file_lock_reports_observed_holder_on_cross_thread_contention() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("lock");
        let owner = NamedFileLock::acquire(&path, "board.append").expect("owner lock");
        let holder_path = named_lock_holder_path(&path);
        let metadata: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&holder_path).unwrap()).unwrap();
        assert_eq!(metadata["pid"], std::process::id());
        assert_eq!(metadata["operation"], "board.append");
        chrono::DateTime::parse_from_rfc3339(metadata["acquired_at"].as_str().unwrap())
            .expect("acquisition timestamp");
        let contender_path = path.clone();
        std::thread::spawn(move || {
            let error = NamedFileLock::try_acquire(&contender_path, "pm.refresh").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
            let message = error.to_string();
            assert!(message.contains("board.append"), "{message}");
            assert!(
                message.contains(&std::process::id().to_string()),
                "{message}"
            );
            assert!(message.contains("acquired_at"), "{message}");
            assert!(
                message.contains("observed"),
                "metadata is observational: {message}"
            );
        })
        .join()
        .unwrap();
        owner.unlock().expect("explicit unlock");
        assert!(path.is_file(), "retain lock inode");
        assert!(!holder_path.exists(), "explicit unlock clears metadata");
        let next = NamedFileLock::try_acquire(&path, "next").expect("released lock is available");
        drop(next);
        assert!(!holder_path.exists(), "drop clears metadata");
    }

    #[test]
    fn named_file_lock_legacy_holder_is_unknown_and_deadline_stays_closed() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("lock");
        let owner = File::create(&path).unwrap();
        owner.lock_exclusive().unwrap();
        let contender_path = path.clone();
        std::thread::spawn(move || {
            let error = NamedFileLock::try_acquire(&contender_path, "try").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
            assert!(error.to_string().contains("unknown"));
            let start = Instant::now();
            let _clock = ScopedOperationClock::set(start);
            let _deadline = ScopedOperationDeadline::enter(start);
            let error = NamedFileLock::acquire(&contender_path, "wait").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::TimedOut);
            assert!(error.to_string().contains("unknown"));
        })
        .join()
        .unwrap();
        FileExt::unlock(&owner).unwrap();
    }

    #[test]
    fn named_file_lock_metadata_failure_does_not_change_ownership() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("lock");
        // A directory makes metadata writes and cleanup fail on every OS.
        std::fs::create_dir(named_lock_holder_path(&path)).unwrap();
        let owner = NamedFileLock::try_acquire(&path, "owner").expect("OS lock acquired");
        let error = NamedFileLock::try_acquire(&path, "contender").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(error.to_string().contains("pid=unknown"));
        owner
            .unlock()
            .expect("metadata failure must not prevent unlock");
        NamedFileLock::try_acquire(&path, "next").expect("OS lock released");
    }

    #[cfg(unix)]
    #[test]
    fn named_file_lock_diagnostics_do_not_follow_symlinks() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("lock");
        let target = directory.path().join("unrelated.json");
        let contents = br#"{"pid":123,"operation":"unrelated","acquired_at":"old"}"#;
        std::fs::write(&target, contents).unwrap();
        std::os::unix::fs::symlink(&target, named_lock_holder_path(&path)).unwrap();
        let legacy = File::create(&path).unwrap();
        legacy.lock_exclusive().unwrap();
        let error = NamedFileLock::try_acquire(&path, "contender").unwrap_err();
        assert!(error.to_string().contains("pid=unknown"), "{error}");
        FileExt::unlock(&legacy).unwrap();
        let owner = NamedFileLock::acquire(&path, "owner").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), contents);
        owner.unlock().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn named_file_lock_diagnostic_fifo_is_not_read() {
        use std::os::unix::{ffi::OsStrExt, fs::OpenOptionsExt};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("holder.fifo");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a live NUL-terminated path in our private directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        // Keep a writer present so a regression in open flags cannot hang the
        // test. The opened-handle check must reject the FIFO before any read.
        let _peer = File::options()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(&path)
            .unwrap();
        assert!(open_named_lock_holder(&path).is_err());
        let error = named_lock_error(
            &path,
            "contender",
            io::Error::new(io::ErrorKind::WouldBlock, "busy"),
        );
        assert!(error.to_string().contains("pid=unknown"));
    }

    #[test]
    fn fixed_operation_clock_controls_expiry_without_spending_wall_time() {
        let directory = tempfile::tempdir().expect("tempdir");
        let file = File::create(directory.path().join("lock")).expect("lock file");
        // This deadline is already expired on the host clock. Only the
        // explicitly advanced operation clock may decide the test's outcome.
        let start = Instant::now() - Duration::from_secs(1);
        let expiry = start + Duration::from_millis(250);
        let _clock = ScopedOperationClock::set(start);
        let _deadline = ScopedOperationDeadline::enter(expiry);
        assert_eq!(now(), start);
        lock_exclusive(&file).expect("logical budget remains");
        FileExt::unlock(&file).expect("unlock");
        ensure_remaining("durable rename").expect("rename remains within budget");

        {
            let _expired = ScopedOperationClock::set(expiry);
            assert_eq!(
                lock_exclusive(&file).unwrap_err().kind(),
                io::ErrorKind::TimedOut
            );
            assert_eq!(
                ensure_remaining("durable rename").unwrap_err().kind(),
                io::ErrorKind::TimedOut
            );
        }
        assert_eq!(now(), start, "nested clock restores the previous instant");
        assert!(std::thread::spawn(now).join().expect("other thread") > start);
        drop(_clock);
        assert!(now() > start, "leaving the scope restores the host clock");
    }

    #[test]
    fn contended_file_lock_stops_at_the_shared_deadline() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("lock");
        let first = File::create(&path).expect("first lock file");
        let second = File::options()
            .read(true)
            .write(true)
            .open(&path)
            .expect("second lock file");
        first.lock_exclusive().expect("hold first lock");
        let started = Instant::now();
        let _deadline = ScopedOperationDeadline::enter(started + Duration::from_millis(40));

        let error = lock_exclusive(&second).expect_err("contended lock must time out");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        FileExt::unlock(&first).expect("unlock first lock");
    }

    #[test]
    fn contended_file_lock_notifies_the_call_scoped_observer() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("lock");
        let first = File::create(&path).expect("first lock file");
        let second = File::options()
            .read(true)
            .write(true)
            .open(&path)
            .expect("second lock file");
        first.lock_exclusive().expect("hold first lock");
        let _deadline = ScopedOperationDeadline::enter(Instant::now() + Duration::from_millis(40));
        let mut observed = 0;

        let error = lock_exclusive_with_observer(&second, || observed += 1)
            .expect_err("contended lock must time out");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(observed >= 1, "the exact call reports lock contention");
        FileExt::unlock(&first).expect("unlock first lock");
    }

    #[test]
    fn expired_deadline_rejects_an_available_file_lock() {
        let directory = tempfile::tempdir().expect("tempdir");
        let path = directory.path().join("lock");
        let file = File::create(&path).expect("lock file");
        let _deadline = ScopedOperationDeadline::enter(Instant::now() - Duration::from_millis(1));

        let error = lock_exclusive(&file).expect_err("expired deadline must reject lock");

        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    }

    #[test]
    fn nested_deadlines_keep_the_earliest_expiry() {
        let outer_expiry = Instant::now() + Duration::from_secs(1);
        let _outer = ScopedOperationDeadline::enter(outer_expiry);
        let _inner = ScopedOperationDeadline::enter(outer_expiry + Duration::from_secs(1));

        assert_eq!(current(), Some(outer_expiry));
    }
}
