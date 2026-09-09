//! Performance regression gate (SPEC #3700 FR-008, Issue #4145 AC-5).
//!
//! FR-008 asks for a same-run relative basis, with absolute values reserved for
//! "obviously abnormal" ceilings. #3699 is the reason: CI machines vary enough
//! that a tight absolute threshold turns into a flaky test rather than a
//! regression signal.
//!
//! So this file gates three things:
//! 1. Budget enforcement itself — deterministic, no timing involved. If
//!    smoothing or the summary stops flagging a sustained overage, this fails.
//! 2. The collector's own cost — a representative burst must stay inside the
//!    normative UI response budget per sample, which is three orders of
//!    magnitude above what the path actually costs.
//! 3. Aggregation scaling — a same-run relative check that catches an
//!    accidentally quadratic `summarize`.

use std::time::{Duration, Instant};

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
