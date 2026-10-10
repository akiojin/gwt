//! Reports a blocked frontend dispatch even if its handler never returns.

use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use gwt_core::error_ledger::{ErrorKind, ErrorRecord};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

const STALL_THRESHOLD: Duration = Duration::from_millis(100);
const PERSIST_THRESHOLD: Duration = Duration::from_secs(5);

struct ActiveDispatch {
    event: String,
    started: Instant,
    started_at: DateTime<Utc>,
    reported: bool,
    persisted: bool,
}

impl ActiveDispatch {
    fn new(event: String, started: Instant) -> Self {
        Self {
            event,
            started,
            started_at: Utc::now(),
            reported: false,
            persisted: false,
        }
    }

    fn report_due(&mut self, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.started);
        if !self.persisted && elapsed >= PERSIST_THRESHOLD {
            // A late first observation consumes both reports, avoiding duplicate rows.
            self.reported = true;
            self.persisted = true;
            return Some(elapsed);
        }
        if self.reported || elapsed <= STALL_THRESHOLD {
            return None;
        }
        self.reported = true;
        Some(elapsed)
    }
}

/// A rare, on-demand process sample, never a second periodic resource poller.
/// The CPU value measures this interval after detection (1 core = 100%).
fn persist_stall(event: &str, elapsed: Duration, started_at: DateTime<Utc>) {
    let detected_at = Utc::now();
    let pid = sysinfo::Pid::from_u32(std::process::id());
    let mut system = System::new();
    let refresh = ProcessRefreshKind::nothing()
        .with_memory()
        .with_cpu()
        .without_tasks();
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, refresh);
    let sample_started = Instant::now();
    thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
    system.refresh_processes_specifics(ProcessesToUpdate::Some(&[pid]), true, refresh);
    let mut context = BTreeMap::from([
        ("event".into(), event.to_owned()),
        ("started_at".into(), started_at.to_rfc3339()),
        ("detected_at".into(), detected_at.to_rfc3339()),
        ("elapsed_ms".into(), elapsed.as_millis().to_string()),
        ("pid".into(), pid.as_u32().to_string()),
        ("version".into(), env!("CARGO_PKG_VERSION").to_owned()),
        ("resources_sampled_at".into(), Utc::now().to_rfc3339()),
        (
            "cpu_sample_ms".into(),
            sample_started.elapsed().as_millis().to_string(),
        ),
    ]);
    if let Some(process) = system.process(pid) {
        context.insert("rss_bytes".into(), process.memory().to_string());
        context.insert("cpu_percent".into(), process.cpu_usage().to_string());
    } else {
        context.insert(
            "resources_unavailable".into(),
            "GUI process was not found".into(),
        );
    }
    let record = ErrorRecord::new_host(
        ErrorKind::GuiEventLoopStall,
        format!(
            "{event} is blocking the GUI event loop ({}ms)",
            elapsed.as_millis()
        ),
    )
    .with_context(context);
    if let Err(error) = gwt_core::error_ledger::record(record) {
        tracing::warn!(%error, "cannot persist GUI dispatch stall");
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
        #[cfg(test)]
        let home = gwt_core::test_support::gwt_home_override();
        Self::with_reporter(move |event, elapsed, started_at| {
            #[cfg(test)]
            let _home = home
                .as_ref()
                .map(gwt_core::test_support::ScopedGwtHome::set);
            if elapsed >= PERSIST_THRESHOLD {
                // Persist before entering tracing's project-router lock.
                persist_stall(event, elapsed, started_at);
            }
            tracing::warn!(
                target: "gwt.frontend.timing",
                event,
                elapsed_ms = elapsed.as_secs_f64() * 1000.0,
                "frontend dispatch still running"
            );
        })
    }

    fn with_reporter(report: impl Fn(&str, Duration, DateTime<Utc>) + Send + 'static) -> Self {
        let shared = Arc::new((Mutex::new(State::default()), Condvar::new()));
        let worker_state = Arc::clone(&shared);
        let worker = thread::Builder::new()
            .name("frontend-dispatch-watchdog".into())
            .spawn(move || {
                let (lock, wake) = &*worker_state;
                let mut state = lock.lock().unwrap();
                while !state.shutdown {
                    if let Some(active) = state.active.as_mut().filter(|active| !active.persisted) {
                        let now = Instant::now();
                        if let Some(elapsed) = active.report_due(now) {
                            let event = active.event.clone();
                            let started_at = active.started_at;
                            // Diagnostics describe the active handler at detection. OS,
                            // logger and ledger work must never delay its GUI guard.
                            drop(state);
                            report(&event, elapsed, started_at);
                            state = lock.lock().unwrap();
                            continue;
                        }
                        let threshold = if active.reported {
                            PERSIST_THRESHOLD
                        } else {
                            STALL_THRESHOLD
                        };
                        let remaining =
                            threshold.saturating_sub(now.saturating_duration_since(active.started));
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
    fn long_stall_is_reported_after_the_initial_timing_warning() {
        let started = Instant::now();
        let mut active = ActiveDispatch::new("TerminalConvergenceTick".into(), started);
        assert!(active
            .report_due(started + Duration::from_millis(101))
            .is_some());
        assert_eq!(
            active.report_due(started + Duration::from_secs(5)),
            Some(Duration::from_secs(5))
        );
        assert_eq!(active.report_due(started + Duration::from_secs(6)), None);
    }

    #[test]
    fn blocked_reporter_does_not_block_dispatch_completion() {
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let (completed_tx, completed_rx) = mpsc::channel();
        let mut watchdog = DispatchWatchdog::with_reporter(move |_, _, _| {
            entered_tx.send(()).unwrap();
            release_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        });
        let guard = watchdog.enter("refresh");
        entered_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let progressed = thread::scope(|scope| {
            scope.spawn(move || {
                drop(guard);
                completed_tx.send(()).unwrap();
            });
            let progressed = completed_rx.recv_timeout(Duration::from_secs(5)).is_ok();
            // Always release the reporter before asserting, including the RED case.
            release_tx.send(()).unwrap();
            progressed
        });
        assert!(
            progressed,
            "diagnostic I/O held the GUI dispatch state lock"
        );
    }

    #[test]
    fn long_live_dispatch_is_persisted_with_process_resources_but_fast_dispatch_is_not() {
        let home = tempfile::tempdir().unwrap();
        let _home = gwt_core::test_support::ScopedGwtHome::set(home.path());
        let mut watchdog = DispatchWatchdog::start();
        drop(watchdog.enter("fast"));
        assert!(gwt_core::error_ledger::list_since(None).unwrap().is_empty());

        let guard = watchdog.enter("TerminalConvergenceTick");
        {
            let mut state = guard.watchdog.shared.0.lock().unwrap();
            let active = state.active.as_mut().unwrap();
            // Advance the observed duration rather than wait for the threshold.
            active.started -= Duration::from_secs(6);
            active.started_at -= chrono::Duration::seconds(6);
        }
        guard.watchdog.shared.1.notify_one();
        let deadline = Instant::now() + Duration::from_secs(10);
        let records = loop {
            let records = gwt_core::error_ledger::list_since(None).unwrap();
            if !records.is_empty() || Instant::now() >= deadline {
                break records;
            }
            thread::sleep(Duration::from_millis(100));
        };
        // The ledger row must exist before the GUI handler returns.
        assert_eq!(records.len(), 1, "missing live dispatch stall record");
        let row = &records[0];
        assert_eq!(row.kind, ErrorKind::GuiEventLoopStall);
        assert_eq!(row.scope, gwt_core::error_ledger::ErrorScope::Host);
        assert_eq!(row.context["event"], "TerminalConvergenceTick");
        assert_eq!(row.context["pid"], std::process::id().to_string());
        assert_eq!(row.context["version"], env!("CARGO_PKG_VERSION"));
        assert!(row.context["elapsed_ms"].parse::<u64>().unwrap() >= 6000);
        assert!(row.context["rss_bytes"].parse::<u64>().unwrap() > 0);
        assert!(row.context["cpu_percent"]
            .parse::<f32>()
            .unwrap()
            .is_finite());
        assert!(row.context["cpu_sample_ms"].parse::<u64>().unwrap() > 0);
        for key in ["started_at", "detected_at", "resources_sampled_at"] {
            chrono::DateTime::parse_from_rfc3339(&row.context[key]).unwrap();
        }
        drop(guard);
        drop(watchdog);
        assert_eq!(gwt_core::error_ledger::list_since(None).unwrap().len(), 1);
    }

    #[test]
    fn reports_a_handler_while_its_guard_is_still_alive() {
        let (tx, rx) = mpsc::channel();
        let mut watchdog = DispatchWatchdog::with_reporter(move |event, elapsed, _| {
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
