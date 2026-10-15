//! Performance regression gate (SPEC #3700 FR-008, Issue #4145 AC-5).
//!
//! FR-008 asks for a same-run relative basis, with absolute values reserved for
//! "obviously abnormal" ceilings. #3699 is the reason: CI machines vary enough
//! that a tight absolute threshold turns into a flaky test rather than a
//! regression signal.
//!
//! So this file gates four things:
//! 1. Budget enforcement itself — deterministic, no timing involved. If
//!    smoothing or the summary stops flagging a sustained overage, this fails.
//! 2. The collector's own cost — a representative burst must stay inside the
//!    normative UI response budget per sample, which is three orders of
//!    magnitude above what the path actually costs.
//! 3. Aggregation scaling — a same-run relative check that catches an
//!    accidentally quadratic `summarize`.
//! 4. Session census — cumulative allocation bytes and an abnormal pass ceiling
//!    for six thousand historical runtime namespaces.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    time::{Duration, Instant},
};

use chrono::{TimeZone as _, Utc};
use gwt::perf::{
    budget::PerfBudgets,
    global::PerfRuntime,
    smoothing::ViolationSmoother,
    summary::{percentile, read_records_from_dir, summarize, PerfFilter, PerfLogRecord},
    PerfRoute,
};
use gwt_config::PerfConfig;
use gwt_core::{paths::gwt_logs_dir, test_support::ScopedGwtHome};

struct CensusCountingAllocator;

#[global_allocator]
static CENSUS_ALLOCATOR: CensusCountingAllocator = CensusCountingAllocator;

thread_local! {
    // Disabled outside the measured pass; parallel tests have separate counters.
    // Count cumulative successful allocation requests, not heap retained after frees.
    static CENSUS_ALLOCATED_BYTES: Cell<Option<usize>> = const { Cell::new(None) };
}

fn record_census_allocation(bytes: usize) {
    let _ = CENSUS_ALLOCATED_BYTES.try_with(|counter| {
        counter.set(counter.get().map(|total| total.saturating_add(bytes)));
    });
}

// SAFETY: every operation delegates its original pointer/layout to System.
// The counter neither allocates nor changes allocation ownership or alignment.
unsafe impl GlobalAlloc for CensusCountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: GlobalAlloc's caller supplies a valid allocation layout.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            record_census_allocation(layout.size());
        }
        ptr
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the same valid layout is passed to System's zeroed allocator.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            record_census_allocation(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: ptr and layout retain System's allocation contract.
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller's original allocation and new size go unchanged to System.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            record_census_allocation(new_size);
        }
        new_ptr
    }
}

/// Issue #5383 AC-2: historical namespaces must not pay for Session TOML decode.
/// One second is the target, while five seconds catches an obviously abnormal
/// pass without making scheduler or filesystem variability a tight CI gate.
#[test]
fn session_census_with_six_thousand_dead_namespaces_stays_within_read_budget() {
    let temp = tempfile::tempdir().expect("isolated census fixture");
    let project = temp.path().join("project");
    let sessions = temp.path().join("sessions");
    std::fs::create_dir_all(&project).expect("fixture project");
    let mut session =
        gwt_agent::Session::new(&project, "census-history", gwt_agent::AgentId::Codex);
    session.id = "dead-history".to_string();
    session.project_state_root = Some(project.clone());
    // A representative ledger is shared: the old path decoded it once per namespace.
    session.launch_args = vec!["x".repeat(5 * 1024)];
    session.save(&sessions).expect("valid Session ledger");
    let mut runtime = gwt_agent::SessionRuntimeState::new(gwt_agent::AgentStatus::Stopped);
    runtime.host_started_at = Some(1);
    let runtime_json = serde_json::to_vec(&runtime).expect("valid runtime JSON");
    for index in 0..6000 {
        // Keep Unix pid_t positive so the probe cannot address a process group.
        let host_pid = i32::MAX as u32 - index;
        assert!(
            !gwt::process::is_host_process_alive(host_pid),
            "fixture namespace must be dead: {host_pid}"
        );
        let path = gwt_agent::runtime_state_path_for_pid(&sessions, host_pid, &session.id);
        std::fs::create_dir_all(path.parent().expect("runtime namespace"))
            .expect("create dead namespace");
        std::fs::write(path, &runtime_json).expect("write runtime JSON");
    }

    CENSUS_ALLOCATED_BYTES.with(|counter| counter.set(Some(0)));
    let started = Instant::now();
    let inventory = gwt::session_inventory::observe_sessions(&project, &sessions);
    let elapsed = started.elapsed();
    let allocated_bytes = CENSUS_ALLOCATED_BYTES
        .with(|counter| counter.replace(None))
        .expect("active allocation measurement");
    eprintln!(
        "6000 dead namespace census: {elapsed:?}, {allocated_bytes} allocated bytes (target <1s, <50MB)"
    );
    assert!(inventory.sessions.is_empty());
    assert!(inventory.uncertainties.is_empty());
    assert!(
        allocated_bytes < 50_000_000,
        "dead namespace census allocation budget exceeded: {allocated_bytes} bytes"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "dead namespace census exceeded the abnormal pass ceiling: {elapsed:?}"
    );
}

fn sample_record(target: &str, value: f64) -> PerfLogRecord {
    PerfLogRecord {
        schema_version: 1,
        record_type: "sample".to_string(),
        timestamp: Utc
            .with_ymd_and_hms(2026, 9, 8, 10, 0, 0)
            .single()
            .expect("valid timestamp"),
        stream: "ui".to_string(),
        target: target.to_string(),
        value,
        unit: "ms".to_string(),
        role: None,
        budget: None,
        consecutive_count: None,
        duration_seconds: None,
        detector_version: None,
        startup: None,
    }
}

/// A sustained overage must be detectable, or every budget in the program is
/// decorative. This is the regression FR-008 exists to prevent.
#[test]
fn a_sustained_budget_overage_still_fails_the_budget_check() {
    let budgets = PerfBudgets::default();
    let budget = PerfRoute::PaneClose.budget_ms(&budgets);
    let over_budget = budget * 4.0;

    let mut smoother = ViolationSmoother::new();
    let base = Utc
        .with_ymd_and_hms(2026, 9, 8, 10, 0, 0)
        .single()
        .expect("valid timestamp");
    let violation = (0..3)
        .filter_map(|step| {
            smoother.observe(
                &PerfRoute::PaneClose.target(),
                over_budget,
                budget,
                base + chrono::Duration::milliseconds(step * 10),
            )
        })
        .next()
        .expect("three consecutive overages must be a violation");
    assert_eq!(violation.budget(), budget);

    let records: Vec<PerfLogRecord> = (0..3)
        .map(|_| sample_record(&PerfRoute::PaneClose.target(), over_budget))
        .collect();
    let summary = summarize(&records, &budgets, None);
    assert!(
        summary.targets[0].over_budget,
        "p95 {} must be reported over the {budget}ms budget",
        summary.targets[0].p95
    );
}

/// An in-budget series must not be reported as a violation, or the gate cries
/// wolf and stops being actionable (FR-006 acceptance scenario 7).
#[test]
fn an_in_budget_series_is_not_flagged() {
    let budgets = PerfBudgets::default();
    let budget = PerfRoute::ProjectSwitch.budget_ms(&budgets);

    let records: Vec<PerfLogRecord> = (0..100)
        .map(|step| {
            sample_record(
                &PerfRoute::ProjectSwitch.target(),
                budget / 2.0 + f64::from(step % 5),
            )
        })
        .collect();

    let summary = summarize(&records, &budgets, None);
    assert!(!summary.targets[0].over_budget);
    assert_eq!(summary.violation_count, 0);
}

/// Issue #4292: each CLI process sees only one measurement per target, so
/// violations must survive real process exits, not only runtime recreation.
#[test]
fn sustained_route_and_operation_overages_survive_separate_processes() {
    const CHILD_HOME: &str = "GWT_PERF_REGRESSION_CHILD_HOME";
    let config = PerfConfig::default();
    if let Some(home) = std::env::var_os(CHILD_HOME) {
        let _gwt_home = ScopedGwtHome::set(std::path::PathBuf::from(home));
        let mut runtime =
            PerfRuntime::appending_to_established_log(&config).expect("child perf runtime");
        assert!(
            runtime.is_enabled(),
            "the parent established the fixture log"
        );
        for route in PerfRoute::ALL {
            let elapsed = Duration::from_secs_f64(route.budget_ms(runtime.budgets()) / 500.0);
            runtime.record_route(route, elapsed);
        }
        runtime.record_operation("issue.view", Duration::from_millis(500), true);
        return;
    }

    let home = tempfile::tempdir().expect("isolated perf HOME");
    let _gwt_home = ScopedGwtHome::set(home.path());
    drop(PerfRuntime::from_config(&config).expect("establish fixture log like the GUI"));
    for _ in 0..3 {
        let output = gwt_core::process::hidden_command(
            std::env::current_exe().expect("current test binary"),
        )
        .args([
            "--exact",
            "perf_regression_test::sustained_route_and_operation_overages_survive_separate_processes",
        ])
        .env(CHILD_HOME, home.path())
        .output()
        .expect("run a separate perf collector process");
        assert!(
            output.status.success(),
            "child collector failed: {}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let records = read_records_from_dir(&gwt_logs_dir().join("perf"), &PerfFilter::default())
        .expect("read cross-process evidence");
    for target in PerfRoute::ALL
        .map(PerfRoute::target)
        .into_iter()
        .chain(["gwtd:issue.view".to_string()])
    {
        let target_records: Vec<_> = records
            .iter()
            .filter(|record| record.target == target)
            .collect();
        assert_eq!(
            target_records
                .iter()
                .filter(|record| record.is_sample())
                .count(),
            3,
            "{target}"
        );
        assert_eq!(
            target_records
                .iter()
                .filter(|record| record.is_violation())
                .count(),
            1,
            "{target}"
        );
        assert!(
            target_records
                .iter()
                .all(|record| record.detector_version == Some(1)),
            "{target}"
        );
    }
}

/// The collector must stay far inside the budget of the routes it measures.
/// Issue #3264 is the precedent: instrumentation that costs as much as the work
/// it observes becomes the perf bug. The ceiling is deliberately the normative
/// UI budget — three orders of magnitude above the real cost — so this fails on
/// a genuine regression and not on a slow CI runner.
#[test]
fn collecting_one_sample_stays_far_inside_the_ui_response_budget() {
    let home = tempfile::tempdir().expect("tempdir");
    let _gwt_home = ScopedGwtHome::set(home.path());
    let mut runtime = PerfRuntime::from_config(&PerfConfig::default()).expect("perf runtime");

    // Warm the daily file open and the first allocation out of the sample.
    runtime.record_route(PerfRoute::PromptSend, Duration::from_millis(1));

    let mut costs = Vec::with_capacity(500);
    for _ in 0..500 {
        let started = Instant::now();
        runtime.record_route(PerfRoute::PromptSend, Duration::from_millis(1));
        costs.push(started.elapsed().as_secs_f64() * 1_000.0);
    }

    let budget = PerfBudgets::default().ui_response_ms;
    let p95 = percentile(&costs, 0.95);
    assert!(
        p95 < budget,
        "collecting one perf sample cost p95 {p95}ms, over the {budget}ms UI response budget"
    );

    let records = read_records_from_dir(&gwt_logs_dir().join("perf"), &PerfFilter::default())
        .expect("read perf records");
    assert!(
        !records.is_empty(),
        "the burst must have persisted samples, otherwise the cost measured nothing"
    );
}

/// Same-run relative basis (FR-008): aggregating ten times the records must not
/// cost anywhere near a hundred times as much. This is the shape of regression
/// an accidental nested scan over targets would introduce.
#[test]
fn summary_aggregation_scales_linearly_with_the_record_count() {
    let budgets = PerfBudgets::default();
    let small: Vec<PerfLogRecord> = (0..2_000)
        .map(|step| {
            sample_record(
                &format!("route:target-{}", step % 200),
                f64::from(step % 97),
            )
        })
        .collect();
    let large: Vec<PerfLogRecord> = (0..20_000)
        .map(|step| {
            sample_record(
                &format!("route:target-{}", step % 200),
                f64::from(step % 97),
            )
        })
        .collect();

    let small_elapsed = time_summarize(&small, &budgets);
    let large_elapsed = time_summarize(&large, &budgets);

    let ratio = large_elapsed.as_secs_f64() / small_elapsed.as_secs_f64().max(1e-6);
    assert!(
        ratio < 40.0,
        "aggregating 10x the records cost {ratio:.1}x as much; \
         summarize must stay linear in the record count"
    );
}

fn time_summarize(records: &[PerfLogRecord], budgets: &PerfBudgets) -> Duration {
    // Three runs, best of, so one scheduler hiccup does not decide the ratio.
    (0..3)
        .map(|_| {
            let started = Instant::now();
            let summary = summarize(records, budgets, None);
            assert!(summary.sample_count > 0);
            started.elapsed()
        })
        .min()
        .expect("at least one run")
}
