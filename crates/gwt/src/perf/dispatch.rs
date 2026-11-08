//! Minute summaries of completed slow GUI handlers (Issue #5538).

use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::mpsc::{self, SyncSender},
    time::{Duration, Instant},
};

pub const SLOW_HANDLER: Duration = Duration::from_millis(50);
const WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DispatchSummary {
    pub pid: u32,
    pub count: usize,
    pub total_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
    pub pending_max: Option<usize>,
    pub pending_unknown_count: usize,
    pub window_ms: f64,
    pub dropped_windows: u64,
    pub window_ended_at: chrono::DateTime<chrono::Utc>,
}

struct DispatchBatch {
    handlers: BTreeMap<String, Vec<(f64, Option<usize>)>>,
    window_ms: f64,
    dropped_windows: u64,
    ended_at: chrono::DateTime<chrono::Utc>,
}

impl DispatchBatch {
    fn summary(&self, event: &str) -> DispatchSummary {
        let samples = self
            .handlers
            .get(event)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let values: Vec<_> = samples.iter().map(|sample| sample.0).collect();
        DispatchSummary {
            pid: std::process::id(),
            count: values.len(),
            total_ms: values.iter().sum(),
            p95_ms: super::summary::percentile(&values, 0.95),
            max_ms: values.iter().copied().fold(0.0, f64::max),
            pending_max: samples.iter().filter_map(|sample| sample.1).max(),
            pending_unknown_count: samples.iter().filter(|sample| sample.1.is_none()).count(),
            window_ms: self.window_ms,
            dropped_windows: self.dropped_windows,
            window_ended_at: self.ended_at,
        }
    }
}

/// The GUI only retains slow samples and transfers one batch per minute. Sorting,
/// serialization, file writes and the process-wide perf mutex stay on the worker.
/// Serial handlers over 50ms limit a minute's samples to fewer than 1,200; the
/// window rolls even when the bounded writer queue is full.
pub struct DispatchCollector {
    started: Instant,
    handlers: BTreeMap<String, Vec<(f64, Option<usize>)>>,
    sender: Option<SyncSender<DispatchBatch>>,
    dropped_windows: u64,
}

impl DispatchCollector {
    pub fn start() -> Self {
        Self::with_enabled(super::global::is_enabled())
    }

    fn with_enabled(enabled: bool) -> Self {
        #[cfg(test)]
        let home = gwt_core::test_support::gwt_home_override();
        let sender = if enabled {
            let (tx, rx) = mpsc::sync_channel::<DispatchBatch>(2);
            std::thread::Builder::new()
                .name("gui-dispatch-perf".into())
                .spawn(move || {
                    #[cfg(test)]
                    let _home = home
                        .as_ref()
                        .map(gwt_core::test_support::ScopedGwtHome::set);
                    // A separate sink keeps slow file I/O from holding GLOBAL:
                    // existing GUI route timers also acquire that mutex.
                    let config = gwt_config::Settings::load().unwrap_or_default().perf;
                    let Ok(mut sink) = super::PerfSink::appending_to_established_log(&config)
                    else {
                        return;
                    };
                    while let Ok(batch) = rx.recv() {
                        if batch.handlers.is_empty() {
                            // Prove the collector is alive even in a minute with no stalls.
                            let _ =
                                sink.append(&super::PerfRecord::dispatch("", batch.summary("")));
                        }
                        for event in batch.handlers.keys() {
                            let record = super::PerfRecord::dispatch(event, batch.summary(event));
                            let _ = sink.append(&record);
                        }
                    }
                })
                .ok()
                .map(|_| tx)
        } else {
            None
        };
        Self::with_sender(Instant::now(), sender)
    }

    fn with_sender(started: Instant, sender: Option<SyncSender<DispatchBatch>>) -> Self {
        Self {
            started,
            handlers: BTreeMap::new(),
            sender,
            dropped_windows: 0,
        }
    }

    pub fn record(&mut self, event: &str, elapsed: Duration, pending: Option<usize>, now: Instant) {
        let Some(sender) = self.sender.as_ref() else {
            return;
        };
        if elapsed > SLOW_HANDLER {
            let value = (elapsed.as_secs_f64() * 1_000.0, pending);
            if let Some(samples) = self.handlers.get_mut(event) {
                samples.push(value);
            } else {
                self.handlers.insert(event.to_owned(), vec![value]);
            }
        }
        let window = now.saturating_duration_since(self.started);
        if window < WINDOW {
            return;
        }
        self.started = now;
        let batch = DispatchBatch {
            handlers: std::mem::take(&mut self.handlers),
            window_ms: window.as_secs_f64() * 1_000.0,
            dropped_windows: self.dropped_windows,
            ended_at: chrono::Utc::now(),
        };
        // Never wait for a slow filesystem/subscriber or for worker shutdown.
        match sender.try_send(batch) {
            Ok(()) => self.dropped_windows = 0,
            Err(mpsc::TrySendError::Full(_)) => self.dropped_windows += 1,
            Err(mpsc::TrySendError::Disconnected(_)) => self.sender = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };

    #[test]
    fn dispatch_collection_cost_is_measured_against_the_same_handler_work() {
        // Same-run measurement, never a tight scheduler-dependent timing gate.
        // The supplied duration is data, rather than a real-time wait.
        let (tx, _rx) = mpsc::sync_channel(2);
        let now = Instant::now();
        let mut collector = DispatchCollector::with_sender(now, Some(tx));
        let mut before = Vec::new();
        let mut after = Vec::new();
        const ITERATIONS: usize = 100_000;
        for round in 0..10 {
            for enabled in if round % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let started = Instant::now();
                for index in 0..ITERATIONS {
                    let mut value = std::hint::black_box(index as u64);
                    for _ in 0..256 {
                        value = std::hint::black_box(
                            value.wrapping_mul(6364136223846793005).wrapping_add(1),
                        );
                    }
                    std::hint::black_box(value);
                    if enabled {
                        collector.record("fast", std::hint::black_box(Duration::ZERO), None, now);
                    }
                }
                let ns = started.elapsed().as_nanos() as f64 / ITERATIONS as f64;
                if enabled {
                    after.push(ns);
                } else {
                    before.push(ns);
                }
            }
        }
        let baseline = super::super::summary::percentile(&before, 0.5);
        let measured = super::super::summary::percentile(&after, 0.5);
        println!(
            "ISSUE5538_OVERHEAD {}",
            serde_json::json!({
                "iterations_per_round": ITERATIONS, "rounds": 10,
                "baseline_ns_per_dispatch": baseline, "instrumented_ns_per_dispatch": measured,
                "delta_ns_per_dispatch": measured - baseline,
                "baseline_rounds_ns": before, "instrumented_rounds_ns": after,
            })
        );
        assert!(collector.handlers.is_empty());
    }

    #[test]
    fn slow_handlers_are_coalesced_with_exact_percentile_and_pending_count() {
        let started = Instant::now();
        let (tx, rx) = mpsc::sync_channel(2);
        let mut collector = DispatchCollector::with_sender(started, Some(tx));
        collector.record("refresh", SLOW_HANDLER, Some(99), started);
        for index in 1..=20 {
            collector.record(
                "refresh",
                Duration::from_millis(100 * index),
                Some(index as usize),
                started,
            );
        }
        collector.record("other", Duration::from_millis(100), None, started);
        assert!(rx.try_recv().is_err(), "no per-handler writes");
        collector.record("fast", Duration::ZERO, Some(0), started + WINDOW);
        let batch = rx.try_recv().unwrap();
        assert_eq!(batch.handlers.len(), 2);
        let summary = batch.summary("refresh");
        assert_eq!(summary.count, 20);
        assert_eq!(summary.total_ms, 21_000.0);
        assert_eq!(summary.p95_ms, 1_900.0);
        assert_eq!(summary.max_ms, 2_000.0);
        assert_eq!(summary.pending_max, Some(20));
        assert_eq!(batch.summary("other").pending_unknown_count, 1);
        collector.record("fast", Duration::ZERO, Some(0), started + WINDOW * 2);
        assert!(
            rx.try_recv().unwrap().handlers.is_empty(),
            "the previous minute is reset; an empty window proves collection is alive"
        );
    }

    #[test]
    fn a_blocked_writer_never_blocks_dispatch_and_reports_dropped_windows() {
        let started = Instant::now();
        let (tx, rx) = mpsc::sync_channel(1);
        let mut collector = DispatchCollector::with_sender(started, Some(tx));
        for minute in 1..=2 {
            collector.record(
                "refresh",
                Duration::from_millis(100),
                Some(4),
                started + WINDOW * minute,
            );
        }
        assert_eq!(rx.try_recv().unwrap().dropped_windows, 0);
        collector.record(
            "refresh",
            Duration::from_millis(100),
            Some(2),
            started + WINDOW * 3,
        );
        assert_eq!(rx.try_recv().unwrap().dropped_windows, 1);
    }

    #[test]
    fn minute_summary_uses_the_existing_perf_sink_and_kill_switch() {
        use crate::perf::{PerfRecord, PerfSink};
        use gwt_config::PerfConfig;
        use gwt_core::{paths::gwt_logs_dir, test_support::ScopedGwtHome};
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let started = Instant::now();
        let (tx, rx) = mpsc::sync_channel(1);
        let mut collector = DispatchCollector::with_sender(started, Some(tx));
        collector.record(
            "refresh",
            Duration::from_millis(150),
            Some(8),
            started + WINDOW,
        );
        let batch = rx.try_recv().unwrap();
        let record = PerfRecord::dispatch("refresh", batch.summary("refresh"));
        let disabled = PerfConfig {
            enabled: false,
            ..PerfConfig::default()
        };
        PerfSink::from_config(&disabled)
            .unwrap()
            .append(&record)
            .unwrap();
        assert!(!gwt_logs_dir().join("perf").exists());
        // Establish the log, then exercise the actual worker's private sink.
        drop(PerfSink::from_config(&PerfConfig::default()).unwrap());
        let mut worker = DispatchCollector::with_enabled(true);
        worker.record(
            "refresh",
            Duration::from_millis(150),
            Some(8),
            worker.started + WINDOW,
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        let row: serde_json::Value = loop {
            let row = std::fs::read_dir(gwt_logs_dir().join("perf"))
                .unwrap()
                .filter_map(Result::ok)
                .filter_map(|file| std::fs::read_to_string(file.path()).ok())
                .find_map(|line| serde_json::from_str(&line).ok());
            if let Some(row) = row {
                break row;
            }
            assert!(
                Instant::now() < deadline,
                "dispatch worker did not write its private perf sink"
            );
            std::thread::sleep(Duration::from_millis(100));
        };
        assert_eq!(row["type"], "gui_dispatch_summary");
        assert_eq!(row["target"], "handler:refresh");
        assert_eq!(row["dispatch"]["count"], 1);
        assert_eq!(row["dispatch"]["pending_max"], 8);
        assert_eq!(row["dispatch"]["window_ms"], 60_000.0);
    }
}
