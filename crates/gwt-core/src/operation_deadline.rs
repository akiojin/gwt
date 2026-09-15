//! Thread-scoped absolute deadlines for synchronous local operations.

use std::{
    cell::Cell,
    fs::File,
    io,
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
