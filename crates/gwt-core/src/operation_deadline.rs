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

pub fn lock_exclusive(file: &File) -> io::Result<()> {
    let Some(deadline) = current() else {
        return FileExt::lock_exclusive(file);
    };
    loop {
        if Instant::now() >= deadline {
            return Err(deadline_error("file lock"));
        }
        match file.try_lock_exclusive() {
            Ok(()) => {
                if Instant::now() >= deadline {
                    FileExt::unlock(file)?;
                    return Err(deadline_error("file lock"));
                }
                return Ok(());
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                let now = Instant::now();
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
    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
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
