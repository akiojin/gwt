//! The process-wide performance collector (Issue #4145 AC-1).
//!
//! Most of the instrumented call sites are free functions on background pools
//! (`prepare_project_open`, `spawn_agent_window_async_with_claim`,
//! `search_project_index_attempt`) or live in the `gwtd` binary, which never
//! sees the GUI's `AppRuntime`. Threading a sink handle through all of them
//! would be invasive churn, so collection follows the established process-wide
//! pattern used by `gwt_core::process_console` and `gwt_core::error_ledger`:
//! one installed singleton plus fail-open free functions.
//!
//! Every entry point here is fail-open. An uninstalled collector, a disabled
//! kill switch, a poisoned lock or a write error is a silent no-op — perf
//! collection must never be able to break the path it measures.

use std::{
    io,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

use chrono::Utc;
use gwt_config::PerfConfig;

use super::{
    budget::PerfBudgets,
    record::{PerfRecord, PerfStream, PerfUnit},
    route::PerfRoute,
    self_budget::SelfBudgetGovernor,
    smoothing::ViolationSmoother,
    PerfSink, OPERATION_ROLE_MUTATION, OPERATION_ROLE_READ, OPERATION_TARGET_PREFIX,
};

static GLOBAL: OnceLock<Mutex<PerfRuntime>> = OnceLock::new();

/// Kill switch, budgets, smoothing and self-budget wired to one sink.
pub struct PerfRuntime {
    sink: PerfSink,
    budgets: PerfBudgets,
    smoother: ViolationSmoother,
    governor: SelfBudgetGovernor,
    last_collection_at: Option<Instant>,
}

impl PerfRuntime {
    /// Build a runtime from the persisted performance settings.
    pub fn from_config(config: &PerfConfig) -> io::Result<Self> {
        Self::with_sink(PerfSink::from_config(config)?, config)
    }

    /// Build a runtime that appends only to an already-established perf log.
    pub fn appending_to_established_log(config: &PerfConfig) -> io::Result<Self> {
        Self::with_sink(PerfSink::appending_to_established_log(config)?, config)
    }

    fn with_sink(sink: PerfSink, config: &PerfConfig) -> io::Result<Self> {
        Ok(Self {
            sink,
            budgets: PerfBudgets::resolve(&config.budgets),
            smoother: ViolationSmoother::new(),
            governor: SelfBudgetGovernor::new(config.self_budget_cpu_percent),
            last_collection_at: None,
        })
    }

    /// Whether collection and persistence are enabled.
    pub fn is_enabled(&self) -> bool {
        self.sink.is_enabled()
    }

    /// The budgets in force for this process.
    pub fn budgets(&self) -> &PerfBudgets {
        &self.budgets
    }

    /// Current self-budget thinning factor.
    pub fn sampling_divisor(&self) -> u32 {
        self.governor.sampling_divisor()
    }

    /// Record one end-to-end route measurement.
    pub fn record_route(&mut self, route: PerfRoute, elapsed: Duration) {
        let budget = route.budget_ms(&self.budgets);
        self.record(
            PerfStream::Ui,
            route.target(),
            None,
            elapsed.as_secs_f64() * 1_000.0,
            budget,
        );
    }

    /// Record one gwtd operation measurement.
    pub fn record_operation(&mut self, operation: &str, elapsed: Duration, read_only: bool) {
        let role = if read_only {
            OPERATION_ROLE_READ
        } else {
            OPERATION_ROLE_MUTATION
        };
        let budget = self.budgets.for_operation(read_only);
        self.record(
            PerfStream::Op,
            format!("{OPERATION_TARGET_PREFIX}{operation}"),
            Some(role),
            elapsed.as_secs_f64() * 1_000.0,
            budget,
        );
    }

    fn record(
        &mut self,
        stream: PerfStream,
        target: String,
        role: Option<&'static str>,
        value_ms: f64,
        budget: f64,
    ) {
        if !self.sink.is_enabled() || !value_ms.is_finite() {
            return;
        }

        let collection_started = Instant::now();
        let wall = self
            .last_collection_at
            .replace(collection_started)
            .map(|previous| collection_started.saturating_duration_since(previous))
            .unwrap_or_default();

        if self.governor.should_sample() {
            let now = Utc::now();
            let sample = with_role(
                PerfRecord::sample(now, stream, &target, value_ms, PerfUnit::Milliseconds),
                role,
            );
            let _ = self.sink.append(&sample);

            if let Some(details) = self.smoother.observe(&target, value_ms, budget, now) {
                let violation = with_role(
                    PerfRecord::violation(
                        now,
                        stream,
                        &target,
                        value_ms,
                        PerfUnit::Milliseconds,
                        details,
                    ),
                    role,
                );
                let _ = self.sink.append(&violation);
            }
        }

        self.governor
            .observe_cost(collection_started.elapsed(), wall);
    }
}

fn with_role(record: PerfRecord, role: Option<&'static str>) -> PerfRecord {
    match role {
        Some(role) => record.with_role(role),
        None => record,
    }
}

/// Install the process-wide collector.
///
/// Returns `false` when a collector is already installed or the sink could not
/// be created. Called once per binary; every later call is a no-op so a test
/// harness cannot accidentally replace a live collector.
pub fn install(config: &PerfConfig) -> bool {
    let Ok(runtime) = PerfRuntime::from_config(config) else {
        return false;
    };
    GLOBAL.set(Mutex::new(runtime)).is_ok()
}

/// Install the collector from `~/.gwt/config.toml`, falling back to defaults.
pub fn install_from_settings() -> bool {
    let settings = gwt_config::Settings::load().unwrap_or_default();
    install(&settings.perf)
}

/// Install a collector that appends only to an already-established perf log.
///
/// The entry point for short-lived processes: `gwtd` measures its own
/// operations when the GUI has already established collection on this HOME, and
/// writes nothing at all otherwise.
pub fn install_appending_to_established_log_from_settings() -> bool {
    let settings = gwt_config::Settings::load().unwrap_or_default();
    let Ok(runtime) = PerfRuntime::appending_to_established_log(&settings.perf) else {
        return false;
    };
    GLOBAL.set(Mutex::new(runtime)).is_ok()
}

/// Whether a collector has been installed in this process.
pub fn is_installed() -> bool {
    GLOBAL.get().is_some()
}

/// Run `action` against the installed collector, if there is one.
fn with_runtime(action: impl FnOnce(&mut PerfRuntime)) {
    let Some(runtime) = GLOBAL.get() else {
        return;
    };
    let Ok(mut runtime) = runtime.lock() else {
        return;
    };
    action(&mut runtime);
}

/// Record one end-to-end route measurement, or do nothing when uninstalled.
pub fn record_route(route: PerfRoute, elapsed: Duration) {
    with_runtime(|runtime| runtime.record_route(route, elapsed));
}

/// Record one gwtd operation measurement, or do nothing when uninstalled.
pub fn record_operation(operation: &str, elapsed: Duration, read_only: bool) {
    with_runtime(|runtime| runtime.record_operation(operation, elapsed, read_only));
}

/// Scope guard recording a route measurement when it drops.
///
/// Instrumented routes are full of early returns and `?` propagation; a guard
/// keeps the measurement honest without restructuring the call site.
pub struct RouteTimer {
    route: PerfRoute,
    started: Instant,
}

impl RouteTimer {
    /// Start timing `route`.
    pub fn start(route: PerfRoute) -> Self {
        Self {
            route,
            started: Instant::now(),
        }
    }
}

impl Drop for RouteTimer {
    fn drop(&mut self) {
        record_route(self.route, self.started.elapsed());
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gwt_core::{paths::gwt_logs_dir, test_support::ScopedGwtHome};

    use super::*;
    use crate::perf::summary::{read_records_from_dir, PerfFilter};

    fn read_all() -> Vec<crate::perf::summary::PerfLogRecord> {
        read_records_from_dir(&gwt_logs_dir().join("perf"), &PerfFilter::default())
            .expect("read perf records")
    }

    #[test]
    fn a_route_measurement_lands_in_the_daily_perf_log() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let mut runtime =
            PerfRuntime::from_config(&PerfConfig::default()).expect("create perf runtime");

        runtime.record_route(PerfRoute::ProjectSwitch, Duration::from_millis(12));

        let records = read_all();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].target, "route:project.switch");
        assert_eq!(records[0].stream, "ui");
        assert_eq!(records[0].unit, "ms");
        assert!((records[0].value - 12.0).abs() < 1.0);
    }

    #[test]
    fn an_operation_measurement_records_its_read_or_mutation_role() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let mut runtime =
            PerfRuntime::from_config(&PerfConfig::default()).expect("create perf runtime");

        runtime.record_operation("issue.view", Duration::from_millis(5), true);
        runtime.record_operation("pr.create", Duration::from_millis(5), false);

        let records = read_all();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].target, "gwtd:issue.view");
        assert_eq!(records[0].stream, "op");
        assert_eq!(records[0].role.as_deref(), Some("read"));
        assert_eq!(records[1].target, "gwtd:pr.create");
        assert_eq!(records[1].role.as_deref(), Some("mutation"));
    }

    #[test]
    fn a_sustained_overage_writes_a_violation_next_to_the_samples() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let mut runtime =
            PerfRuntime::from_config(&PerfConfig::default()).expect("create perf runtime");

        for _ in 0..3 {
            runtime.record_route(PerfRoute::PaneClose, Duration::from_millis(400));
        }

        let records = read_all();
        assert_eq!(
            records.iter().filter(|record| record.is_sample()).count(),
            3
        );
        let violations: Vec<_> = records
            .iter()
            .filter(|record| record.is_violation())
            .collect();
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].target, "route:pane.close");
        assert_eq!(violations[0].budget, Some(100.0));
        assert_eq!(violations[0].consecutive_count, Some(3));
    }

    #[test]
    fn a_single_spike_records_a_sample_but_no_violation() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let mut runtime =
            PerfRuntime::from_config(&PerfConfig::default()).expect("create perf runtime");

        runtime.record_route(PerfRoute::PaneClose, Duration::from_millis(400));

        let records = read_all();
        assert_eq!(records.len(), 1);
        assert!(records[0].is_sample());
    }

    /// Issue #4145: a short-lived `gwtd` invocation must leave a HOME the GUI
    /// has never collected in byte-identical. `workspace_cli_test.rs` asserts
    /// exactly this for every forwarded `workspace.update`, so the append-only
    /// sink is what keeps that contract intact.
    #[test]
    fn the_append_only_sink_never_establishes_the_perf_log() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let mut runtime = PerfRuntime::appending_to_established_log(&PerfConfig::default())
            .expect("perf runtime");

        runtime.record_operation("workspace.update", Duration::from_millis(35), false);
        runtime.record_route(PerfRoute::ProjectSwitch, Duration::from_millis(12));

        assert!(!runtime.is_enabled());
        assert!(
            !gwt_logs_dir().join("perf").exists(),
            "gwtd must never bring the perf log into existence"
        );
    }

    /// Once the GUI has established collection, the same append-only sink does
    /// record — otherwise the `op` stream would be permanently empty.
    #[test]
    fn the_append_only_sink_records_into_an_established_perf_log() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        fs::create_dir_all(gwt_logs_dir().join("perf")).expect("establish perf log");
        let mut runtime = PerfRuntime::appending_to_established_log(&PerfConfig::default())
            .expect("perf runtime");

        runtime.record_operation("issue.view", Duration::from_millis(9), true);

        assert!(runtime.is_enabled());
        let records = read_all();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].target, "gwtd:issue.view");
        assert_eq!(records[0].role.as_deref(), Some("read"));
    }

    #[test]
    fn the_kill_switch_stops_collection_without_creating_the_log_directory() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let mut runtime = PerfRuntime::from_config(&PerfConfig {
            enabled: false,
            ..PerfConfig::default()
        })
        .expect("create disabled perf runtime");

        runtime.record_route(PerfRoute::Startup, Duration::from_millis(900));
        runtime.record_operation("issue.view", Duration::from_millis(5), true);

        assert!(!runtime.is_enabled());
        assert!(!gwt_logs_dir().join("perf").exists());
    }

    #[test]
    fn configured_budget_overrides_reach_the_recorded_violation() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let mut config = PerfConfig::default();
        config.budgets.ui_response_ms = Some(10.0);
        let mut runtime = PerfRuntime::from_config(&config).expect("create perf runtime");

        for _ in 0..3 {
            runtime.record_route(PerfRoute::ProjectSwitch, Duration::from_millis(50));
        }

        let violation = read_all()
            .into_iter()
            .find(|record| record.is_violation())
            .expect("override budget must be enforced");
        assert_eq!(violation.budget, Some(10.0));
    }

    #[test]
    fn collection_stays_inside_its_own_cpu_budget_over_a_burst() {
        let home = tempfile::tempdir().expect("tempdir");
        let _gwt_home = ScopedGwtHome::set(home.path());
        let mut runtime =
            PerfRuntime::from_config(&PerfConfig::default()).expect("create perf runtime");

        let started = Instant::now();
        for _ in 0..2_000 {
            runtime.record_route(PerfRoute::PromptSend, Duration::from_millis(1));
        }
        let elapsed = started.elapsed();

        let perf_dir = gwt_logs_dir().join("perf");
        assert!(perf_dir.exists());
        assert!(
            fs::read_dir(&perf_dir).expect("read perf dir").count() >= 1,
            "the burst must have produced a daily log"
        );
        assert!(
            elapsed < Duration::from_secs(20),
            "2000 samples took {elapsed:?}; collection must stay cheap"
        );
    }
}
