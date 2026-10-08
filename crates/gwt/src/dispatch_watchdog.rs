//! Reports a blocked frontend dispatch even if its handler never returns.

use std::{
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const STALL_THRESHOLD: Duration = Duration::from_millis(100);

struct ActiveDispatch {
    event: String,
    started: Instant,
    reported: bool,
}

impl ActiveDispatch {
    fn new(event: String, started: Instant) -> Self {
        Self {
            event,
            started,
            reported: false,
        }
    }

    fn report_due(&mut self, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.started);
        if self.reported || elapsed <= STALL_THRESHOLD {
            return None;
        }
        self.reported = true;
        Some(elapsed)
    }
}

#[derive(Default)]
struct State {
    active: Option<ActiveDispatch>,
    shutdown: bool,
}

pub(crate) struct DispatchWatchdog {
    shared: Arc<(Mutex<State>, Condvar)>,
    worker: Option<JoinHandle<()>>,
}

impl DispatchWatchdog {
    pub(crate) fn start() -> Self {
        Self::with_reporter(|event, elapsed| {
            tracing::warn!(
                target: "gwt.frontend.timing",
                event,
                elapsed_ms = elapsed.as_secs_f64() * 1000.0,
                "frontend dispatch still running"
            );
        })
    }

    fn with_reporter(report: impl Fn(&str, Duration) + Send + 'static) -> Self {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker_state = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("frontend-dispatch-watchdog".into())
            .spawn(move || {
                let (lock, wake) = &*worker_state;
                let mut state = lock.lock().unwrap();
                while !state.shutdown {
                    if let Some(active) = state.active.as_mut().filter(|active| !active.reported) {
                        let now = Instant::now();
                        if let Some(elapsed) = active.report_due(now) {
                            // Keep completion serialized with reporting: the handler is
                            // still active when the warning is emitted.
                            report(&active.event, elapsed);
                            continue;
                        }
                        let remaining = STALL_THRESHOLD
                            .saturating_sub(now.saturating_duration_since(active.started));
                        state = wake.wait_timeout(state, remaining).unwrap().0;
                    } else {
                        state = wake.wait(state).unwrap();
                    }
                }
            })
            .expect("spawn frontend dispatch watchdog");
        Self {
            shared,
            worker: Some(worker),
        }
    }

    // The mutable borrow prevents overlapping dispatch guards on this event loop.
    pub(crate) fn enter(&mut self, event: impl Into<String>) -> DispatchGuard<'_> {
        let (lock, wake) = &*self.shared;
        lock.lock().unwrap().active = Some(ActiveDispatch::new(event.into(), Instant::now()));
        wake.notify_one();
        DispatchGuard { watchdog: self }
    }
}

impl Drop for DispatchWatchdog {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        lock.lock().unwrap().shutdown = true;
        wake.notify_one();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(crate) struct DispatchGuard<'a> {
    watchdog: &'a DispatchWatchdog,
}

impl DispatchGuard<'_> {
    /// A queued completion can unwrap to a more specific handler on this dispatch.
    pub(crate) fn set_event(&mut self, event: &str) {
        if let Some(active) = self.watchdog.shared.0.lock().unwrap().active.as_mut() {
            active.event = event.to_owned();
        }
    }
}

impl Drop for DispatchGuard<'_> {
    fn drop(&mut self) {
        let (lock, wake) = &*self.watchdog.shared;
        lock.lock().unwrap().active = None;
        wake.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn threshold_reports_once_and_only_after_one_hundred_ms() {
        let started = Instant::now();
        let mut active = ActiveDispatch::new("refresh".into(), started);
        assert_eq!(active.report_due(started), None);
        assert_eq!(active.report_due(started + STALL_THRESHOLD), None);
        assert_eq!(
            active.report_due(started + Duration::from_millis(101)),
            Some(Duration::from_millis(101))
        );
        assert_eq!(active.report_due(started + Duration::from_secs(1)), None);
    }

    #[test]
    fn reports_a_handler_while_its_guard_is_still_alive() {
        let (tx, rx) = mpsc::channel();
        let mut watchdog = DispatchWatchdog::with_reporter(move |event, elapsed| {
            tx.send((event.to_owned(), elapsed)).unwrap();
        });
        let mut guard = watchdog.enter("refresh");
        let (event, elapsed) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(event, "refresh");
        assert!(elapsed > STALL_THRESHOLD);
        guard.set_event("completion");
        assert_eq!(
            guard
                .watchdog
                .shared
                .0
                .lock()
                .unwrap()
                .active
                .as_ref()
                .unwrap()
                .event,
            "completion"
        );
        drop(guard);
        assert!(watchdog.shared.0.lock().unwrap().active.is_none());
        drop(watchdog);
        assert!(rx.try_recv().is_err());
    }
}
