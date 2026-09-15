//! One process startup, including overlapping work and input readiness (#3808).
//! Samples use the existing perf sink; absent milestones remain absent.

use std::{
    collections::{HashMap, HashSet},
    sync::{Mutex, OnceLock},
    time::Instant,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{summary::PerfLogRecord, PerfRecord};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupPhase {
    ProcessStart,
    RuntimeInit,
    WorkspaceRestore,
    ProjectStateLoad,
    BrowserInit,
    CanvasReady,
    FirstFrame,
    RestoreDrain,
    PtyStart,
    IndexRuntimeReady,
    ShellInteractive,
    TerminalInteractive,
}

impl StartupPhase {
    pub fn name(self) -> &'static str {
        match self {
            Self::ProcessStart => "process_start",
            Self::RuntimeInit => "runtime_init",
            Self::WorkspaceRestore => "workspace_restore",
            Self::ProjectStateLoad => "project_state_load",
            Self::BrowserInit => "browser_init",
            Self::CanvasReady => "canvas_ready",
            Self::FirstFrame => "first_frame",
            Self::RestoreDrain => "restore_drain",
            Self::PtyStart => "pty_start",
            Self::IndexRuntimeReady => "index_runtime_ready",
            Self::ShellInteractive => "shell_interactive",
            Self::TerminalInteractive => "terminal_interactive",
        }
    }

    const ALL: [Self; 12] = [
        Self::ProcessStart,
        Self::RuntimeInit,
        Self::WorkspaceRestore,
        Self::ProjectStateLoad,
        Self::BrowserInit,
        Self::CanvasReady,
        Self::FirstFrame,
        Self::RestoreDrain,
        Self::PtyStart,
        Self::IndexRuntimeReady,
        Self::ShellInteractive,
        Self::TerminalInteractive,
    ];
}

/// Metadata attached to a perf row, never terminal bytes or project paths.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StartupSample {
    pub startup_id: String,
    pub process_started_at: DateTime<Utc>,
    pub phase: StartupPhase,
    pub start_ms: f64,
    pub restored_window_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
}

struct TerminalReadiness {
    started_ms: f64,
    restored: bool,
    frontend_ready: bool,
    pty_ready: bool,
}

/// Clock-independent state, so readiness ordering and budgets are testable.
pub struct StartupRun {
    id: String,
    started_at: DateTime<Utc>,
    restored_window_count: usize,
    seen: HashSet<StartupPhase>,
    terminals: HashMap<String, TerminalReadiness>,
}

impl StartupRun {
    pub fn new(started_at: DateTime<Utc>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            started_at,
            restored_window_count: 0,
            seen: HashSet::new(),
            terminals: HashMap::new(),
        }
    }

    pub fn set_restored_window_count(&mut self, count: usize) {
        self.restored_window_count = count;
    }

    pub fn phase(
        &mut self,
        phase: StartupPhase,
        start_ms: f64,
        duration_ms: f64,
    ) -> Option<PerfRecord> {
        if !start_ms.is_finite()
            || !duration_ms.is_finite()
            || start_ms < 0.0
            || duration_ms < 0.0
            || self.seen.contains(&phase)
        {
            return None;
        }
        self.seen.insert(phase);
        Some(self.sample(phase, start_ms, duration_ms, None))
    }

    fn sample(
        &self,
        phase: StartupPhase,
        start_ms: f64,
        duration_ms: f64,
        window_id: Option<String>,
    ) -> PerfRecord {
        PerfRecord::startup(
            StartupSample {
                startup_id: self.id.clone(),
                process_started_at: self.started_at,
                phase,
                start_ms,
                restored_window_count: self.restored_window_count,
                window_id,
            },
            duration_ms,
        )
    }

    pub fn track_terminal(&mut self, id: &str, started_ms: f64) {
        self.track_startup_terminal(id, started_ms, true);
    }

    fn track_startup_terminal(&mut self, id: &str, started_ms: f64, restored: bool) {
        // Later project opens and user launches are not part of this startup.
        if self.seen.contains(&StartupPhase::RestoreDrain) {
            return;
        }
        self.terminals
            .entry(id.to_owned())
            .or_insert(TerminalReadiness {
                started_ms,
                restored,
                frontend_ready: false,
                pty_ready: false,
            });
        // An automatically resumed PM can be admitted by the PM queue rather
        // than the general restore queue. Include that extra restored pane.
        self.restored_window_count = self.restored_window_count.max(
            self.terminals
                .values()
                .filter(|terminal| terminal.restored)
                .count(),
        );
    }

    pub fn terminal_ready(&mut self, id: &str, elapsed_ms: f64) -> Vec<PerfRecord> {
        let Some(terminal) = self.terminals.get_mut(id) else {
            return Vec::new();
        };
        terminal.frontend_ready = true;
        self.record_interactive(id, elapsed_ms)
            .into_iter()
            .collect()
    }

    pub fn pty_ready(&mut self, id: &str, elapsed_ms: f64) -> Vec<PerfRecord> {
        let Some(terminal) = self.terminals.get_mut(id) else {
            return Vec::new();
        };
        if terminal.pty_ready {
            return Vec::new();
        }
        terminal.pty_ready = true;
        let start_ms = terminal.started_ms;
        self.seen.insert(StartupPhase::PtyStart);
        let mut records = vec![self.sample(
            StartupPhase::PtyStart,
            start_ms,
            (elapsed_ms - start_ms).max(0.0),
            Some(super::sanitize_ui_action_field(id)),
        )];
        records.extend(self.record_interactive(id, elapsed_ms));
        records
    }

    fn record_interactive(&mut self, id: &str, elapsed_ms: f64) -> Option<PerfRecord> {
        let terminal = self.terminals.get(id)?;
        if !terminal.frontend_ready || !terminal.pty_ready {
            return None;
        }
        self.phase(StartupPhase::TerminalInteractive, 0.0, elapsed_ms)
    }

    pub fn forget_terminal(&mut self, id: &str) {
        self.terminals.remove(id);
    }
}

struct TimedRun {
    started: Instant,
    run: StartupRun,
}
static STARTUP: OnceLock<Mutex<TimedRun>> = OnceLock::new();

/// Install after the perf sink, retaining the instant captured at main entry.
pub fn begin(started: Instant) {
    let started_at = Utc::now() - chrono::Duration::from_std(started.elapsed()).unwrap_or_default();
    if STARTUP
        .set(Mutex::new(TimedRun {
            started,
            run: StartupRun::new(started_at),
        }))
        .is_ok()
    {
        record(StartupPhase::ProcessStart, started, 0.0);
    }
}

fn update(action: impl FnOnce(&mut StartupRun, f64) -> Vec<PerfRecord>) {
    let Some(state) = STARTUP.get() else {
        return;
    };
    let records = {
        let Ok(mut state) = state.lock() else {
            return;
        };
        let elapsed_ms = state.started.elapsed().as_secs_f64() * 1_000.0;
        action(&mut state.run, elapsed_ms)
    };
    for record in records {
        super::global::record_startup_sample(&record);
    }
}

fn record(phase: StartupPhase, started: Instant, duration_ms: f64) {
    let Some(state) = STARTUP.get() else {
        return;
    };
    let offset = match state.lock() {
        Ok(state) => {
            started
                .saturating_duration_since(state.started)
                .as_secs_f64()
                * 1_000.0
        }
        Err(_) => return,
    };
    update(|run, _| run.phase(phase, offset, duration_ms).into_iter().collect());
}

pub fn mark(phase: StartupPhase) {
    update(|run, elapsed| run.phase(phase, 0.0, elapsed).into_iter().collect());
}

pub fn first_frame(navigation_ms: f64) {
    update(|run, elapsed| {
        let mut records = Vec::new();
        if navigation_ms.is_finite() && navigation_ms >= 0.0 {
            records.extend(run.phase(
                StartupPhase::BrowserInit,
                (elapsed - navigation_ms).max(0.0),
                navigation_ms,
            ));
        }
        records.extend(run.phase(StartupPhase::FirstFrame, 0.0, elapsed));
        records
    });
}

pub fn set_restored_window_count(count: usize) {
    update(|run, _| {
        run.set_restored_window_count(count);
        Vec::new()
    });
}

pub fn track_terminal(id: &str) {
    update(|run, elapsed| {
        run.track_terminal(id, elapsed);
        Vec::new()
    });
}

/// A fresh automatic PM is part of startup input readiness, but not a restore.
pub fn track_new_terminal(id: &str) {
    update(|run, elapsed| {
        run.track_startup_terminal(id, elapsed, false);
        Vec::new()
    });
}

pub fn terminal_ready(id: &str) {
    update(|run, elapsed| run.terminal_ready(id, elapsed));
}
pub fn pty_ready(id: &str) {
    update(|run, elapsed| run.pty_ready(id, elapsed));
}
pub fn forget_terminal(id: &str) {
    update(|run, _| {
        run.forget_terminal(id);
        Vec::new()
    });
}

pub struct PhaseTimer {
    phase: StartupPhase,
    started: Instant,
}
impl PhaseTimer {
    pub fn start(phase: StartupPhase) -> Self {
        Self {
            phase,
            started: Instant::now(),
        }
    }
}
impl Drop for PhaseTimer {
    fn drop(&mut self) {
        record(
            self.phase,
            self.started,
            self.started.elapsed().as_secs_f64() * 1_000.0,
        );
    }
}

#[derive(Debug, Serialize)]
pub struct StartupPhaseResult {
    pub phase: StartupPhase,
    pub start_ms: f64,
    pub duration_ms: f64,
    pub end_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct StartupReport {
    pub startup_id: String,
    pub process_started_at: DateTime<Utc>,
    pub restored_window_count: usize,
    pub phases: Vec<StartupPhaseResult>,
    pub missing_phases: Vec<StartupPhase>,
    pub first_frame_budget_ms: f64,
    pub first_frame_within_budget: Option<bool>,
}

pub fn latest_startup(records: &[PerfLogRecord]) -> Option<StartupReport> {
    let newest = records
        .iter()
        .filter_map(|record| record.startup.as_ref())
        .max_by_key(|sample| sample.process_started_at)?;
    let samples = records
        .iter()
        .filter_map(|record| {
            let sample = record.startup.as_ref()?;
            (record.record_type == "startup_phase"
                && sample.startup_id == newest.startup_id
                && record.value.is_finite()
                && record.value >= 0.0
                && sample.start_ms.is_finite()
                && sample.start_ms >= 0.0)
                .then_some((record, sample))
        })
        .collect::<Vec<_>>();
    let restored_window_count = samples
        .iter()
        .map(|(_, sample)| sample.restored_window_count)
        .max()
        .unwrap_or(0);
    let phases = samples
        .iter()
        .map(|(record, sample)| StartupPhaseResult {
            phase: sample.phase,
            start_ms: sample.start_ms,
            duration_ms: record.value,
            end_ms: sample.start_ms + record.value,
            window_id: sample.window_id.clone(),
        })
        .collect::<Vec<_>>();
    let first_frame_budget_ms = if restored_window_count == 0 {
        2_000.0
    } else {
        5_000.0
    };
    let first_frame_within_budget = phases
        .iter()
        .find(|phase| phase.phase == StartupPhase::FirstFrame)
        .map(|phase| phase.end_ms <= first_frame_budget_ms);
    let missing_phases = StartupPhase::ALL
        .into_iter()
        .filter(|phase| !phases.iter().any(|present| present.phase == *phase))
        .collect();
    Some(StartupReport {
        startup_id: newest.startup_id.clone(),
        process_started_at: newest.process_started_at,
        restored_window_count,
        phases,
        missing_phases,
        first_frame_budget_ms,
        first_frame_within_budget,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn first_input_can_be_ready_in_a_new_pm_before_restored_terminals() {
        let mut run = super::StartupRun::new(chrono::Utc::now());
        run.track_terminal("restored", 0.0);
        run.track_startup_terminal("new-pm", 10.0, false);
        assert!(run.terminal_ready("new-pm", 20.0).is_empty());
        let records = run.pty_ready("new-pm", 30.0);
        let report =
            super::latest_startup(&records.into_iter().map(read_record).collect::<Vec<_>>())
                .unwrap();
        assert_eq!(report.restored_window_count, 1);
        assert_eq!(
            report
                .phases
                .iter()
                .find(|p| p.phase == super::StartupPhase::TerminalInteractive)
                .unwrap()
                .end_ms,
            30.0,
        );
    }

    use crate::perf::{
        startup::{latest_startup, StartupPhase, StartupRun},
        summary::{read_records, PerfFilter, PerfLogRecord},
        PerfRecord, PerfSink,
    };
    use chrono::{TimeZone, Utc};
    use gwt_config::PerfConfig;
    use gwt_core::test_support::ScopedGwtHome;

    fn read_record(record: PerfRecord) -> PerfLogRecord {
        serde_json::from_value(serde_json::to_value(record).unwrap()).unwrap()
    }

    #[test]
    fn startup_roundtrip_keeps_phase_offsets_restore_count_and_frame_budget() {
        let home = tempfile::tempdir().unwrap();
        let _home = ScopedGwtHome::set(home.path());
        let mut sink = PerfSink::from_config(&PerfConfig::default()).unwrap();
        let mut run = StartupRun::new(Utc.timestamp_opt(100, 0).unwrap());
        run.set_restored_window_count(3);
        sink.append(&run.phase(StartupPhase::RuntimeInit, 50.0, 20.0).unwrap())
            .unwrap();
        sink.append(&run.phase(StartupPhase::FirstFrame, 0.0, 4_900.0).unwrap())
            .unwrap();

        let report = latest_startup(&read_records(&PerfFilter::default()).unwrap()).unwrap();
        assert_eq!(report.restored_window_count, 3);
        assert_eq!(report.first_frame_budget_ms, 5_000.0);
        assert_eq!(report.first_frame_within_budget, Some(true));
        let runtime = report
            .phases
            .iter()
            .find(|p| p.phase == StartupPhase::RuntimeInit)
            .unwrap();
        assert_eq!(
            (runtime.start_ms, runtime.duration_ms, runtime.end_ms),
            (50.0, 20.0, 70.0)
        );
        assert!(report
            .missing_phases
            .contains(&StartupPhase::IndexRuntimeReady));
    }

    #[test]
    fn latest_startup_excludes_late_samples_from_an_older_process_and_marks_missing_frame() {
        let mut old = StartupRun::new(Utc.timestamp_opt(100, 0).unwrap());
        let mut new = StartupRun::new(Utc.timestamp_opt(200, 0).unwrap());
        let mut records = vec![
            read_record(old.phase(StartupPhase::FirstFrame, 0.0, 100.0).unwrap()),
            read_record(new.phase(StartupPhase::ProcessStart, 0.0, 0.0).unwrap()),
            read_record(
                old.phase(StartupPhase::IndexRuntimeReady, 0.0, 150_000.0)
                    .unwrap(),
            ),
        ];
        let report = latest_startup(&records).unwrap();
        assert_eq!(report.first_frame_within_budget, None);
        assert!(report.missing_phases.contains(&StartupPhase::FirstFrame));
        records.push(read_record(
            new.phase(StartupPhase::FirstFrame, 0.0, 2_001.0).unwrap(),
        ));
        let report = latest_startup(&records).unwrap();
        assert_eq!(report.first_frame_budget_ms, 2_000.0);
        assert_eq!(report.first_frame_within_budget, Some(false));
    }

    #[test]
    fn terminal_interactive_requires_frontend_and_pty_in_either_order_without_waiting_for_a_key() {
        for frontend_first in [true, false] {
            let mut run = StartupRun::new(Utc::now());
            run.track_terminal("restored", 100.0);
            let first = if frontend_first {
                run.terminal_ready("restored", 200.0)
            } else {
                run.pty_ready("restored", 200.0)
            };
            assert!(!first
                .iter()
                .map(|r| serde_json::to_value(r).unwrap())
                .any(|r| { r["startup"]["phase"] == "terminal_interactive" }));
            let second = if frontend_first {
                run.pty_ready("restored", 300.0)
            } else {
                run.terminal_ready("restored", 300.0)
            };
            let records = first
                .into_iter()
                .chain(second)
                .map(read_record)
                .collect::<Vec<_>>();
            let report = latest_startup(&records).unwrap();
            let interactive = report
                .phases
                .iter()
                .find(|p| p.phase == StartupPhase::TerminalInteractive)
                .unwrap();
            assert_eq!(interactive.end_ms, 300.0);
            assert!(run.terminal_ready("restored", 400.0).is_empty());
            assert!(run.terminal_ready("unrelated", 400.0).is_empty());
        }
    }
}
