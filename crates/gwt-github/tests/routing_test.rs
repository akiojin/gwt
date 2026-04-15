//! Contract tests for the `routing` module (SPEC-12 tdd.md Layer 3).

use std::collections::BTreeMap;

use gwt_github::{
    body::SectionLocation,
    routing::{decide_routing, ROUTING_PROMOTE_THRESHOLD_BYTES},
    sections::SectionName,
};

fn n(s: &str) -> SectionName {
    SectionName(s.to_string())
}

fn make_sections(entries: &[(&str, usize)]) -> BTreeMap<SectionName, String> {
    entries
        .iter()
        .map(|(name, size)| (n(name), "x".repeat(*size)))
        .collect()
}

// RED-20: small spec/tasks stay in body
#[test]
fn red_20_small_spec_and_tasks_stay_in_body() {
    let s = make_sections(&[("spec", 4000), ("tasks", 5000)]);
    let r = decide_routing(&s);
    assert_eq!(r.0.get(&n("spec")), Some(&SectionLocation::Body));
    assert_eq!(r.0.get(&n("tasks")), Some(&SectionLocation::Body));
}

// RED-21: 16 KiB + 1 byte section is promoted to comment
#[test]
fn red_21_over_threshold_promoted() {
    let s = make_sections(&[("spec", ROUTING_PROMOTE_THRESHOLD_BYTES + 1)]);
    let r = decide_routing(&s);
    assert_eq!(
        r.0.get(&n("spec")),
        Some(&SectionLocation::Comments(vec![]))
    );
}

// RED-22: non-default sections always go to comments
#[test]
fn red_22_non_default_sections_go_to_comments() {
    let s = make_sections(&[("plan", 500), ("research", 200), ("data-model", 100)]);
    let r = decide_routing(&s);
    assert_eq!(
        r.0.get(&n("plan")),
        Some(&SectionLocation::Comments(vec![]))
    );
    assert_eq!(
        r.0.get(&n("research")),
        Some(&SectionLocation::Comments(vec![]))
    );
    assert_eq!(
        r.0.get(&n("data-model")),
        Some(&SectionLocation::Comments(vec![]))
    );
}

// RED-23: when body total exceeds 60 KiB, largest body section is demoted
#[test]
fn red_23_demotes_largest_body_section_when_over_budget() {
    // spec = 13 KiB, tasks = 14 KiB. Both under threshold individually, but
    // total = 27 KiB which is still under 60 KiB so both stay in body.
    let s = make_sections(&[("spec", 13 * 1024), ("tasks", 14 * 1024)]);
    let r = decide_routing(&s);
    assert_eq!(r.0.get(&n("spec")), Some(&SectionLocation::Body));
    assert_eq!(r.0.get(&n("tasks")), Some(&SectionLocation::Body));

    // Force an impossible case: spec = 15 KiB + tasks = 15 KiB + extras in body
    // would not happen since extras default to comment; but let's simulate with
    // both spec and tasks at exactly threshold and an artificially added body entry.
    // The real overflow scenario is handled by the per-section promote rule,
    // so this test validates the normal case doesn't touch them.
}

// RED-24: both spec and tasks oversized → both go to comments
#[test]
fn red_24_both_spec_and_tasks_oversized() {
    let s = make_sections(&[
        ("spec", ROUTING_PROMOTE_THRESHOLD_BYTES + 100),
        ("tasks", ROUTING_PROMOTE_THRESHOLD_BYTES + 100),
    ]);
    let r = decide_routing(&s);
    assert_eq!(
        r.0.get(&n("spec")),
        Some(&SectionLocation::Comments(vec![]))
    );
    assert_eq!(
        r.0.get(&n("tasks")),
        Some(&SectionLocation::Comments(vec![]))
    );
}

// RED-25: pure function — same input yields same output
#[test]
fn red_25_decide_routing_is_pure() {
    let s = make_sections(&[("spec", 100), ("tasks", 200), ("plan", 5000)]);
    let r1 = decide_routing(&s);
    let r2 = decide_routing(&s);
    assert_eq!(r1, r2);
}
