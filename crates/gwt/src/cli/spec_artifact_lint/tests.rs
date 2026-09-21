//! Tests for the deterministic SPEC artifact lint (Issue #4541 AC-1 / AC-2).

#![cfg(test)]

use chrono::TimeZone;

use super::*;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 21, 12, 0, 0).unwrap()
}

fn section(name: &str, content: &str) -> SectionInput {
    SectionInput {
        name: name.to_string(),
        content: content.to_string(),
    }
}

fn input(sections: Vec<SectionInput>) -> ArtifactInput {
    ArtifactInput {
        owner_number: 4541,
        sections,
        marker_header: None,
        roundtrip: Vec::new(),
    }
}

fn codes(report: &LintReport) -> Vec<LintCode> {
    report.findings.iter().map(|finding| finding.code).collect()
}

// --- AC-1: numbering continuity and duplicates -------------------------

#[test]
fn clean_sequential_numbering_produces_no_findings() {
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one\n- **FR-002**: two\n- **FR-003**: three\n",
        )]),
        now(),
    );
    assert!(
        report.is_clean(),
        "unexpected findings: {:?}",
        report.findings
    );
    assert_eq!(report.sections_scanned, vec!["spec".to_string()]);
}

#[test]
fn duplicate_requirement_number_is_a_critical_finding() {
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one\n- **FR-002**: two\n- **FR-002**: two again\n",
        )]),
        now(),
    );
    let duplicates = report.by_code(LintCode::NumberingDuplicate);
    assert_eq!(duplicates.len(), 1, "findings: {:?}", report.findings);
    assert_eq!(duplicates[0].refs, vec!["FR-002".to_string()]);
    assert_eq!(duplicates[0].severity, LintSeverity::Critical);
    assert!(
        duplicates[0].message.contains("spec:2"),
        "message should name both sites: {}",
        duplicates[0].message
    );
}

#[test]
fn numbering_gap_is_reported_as_one_finding_per_run() {
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one\n- **FR-004**: four\n- **FR-005**: five\n- **FR-008**: eight\n",
        )]),
        now(),
    );
    let gaps = report.by_code(LintCode::NumberingGap);
    assert_eq!(gaps.len(), 2, "findings: {:?}", report.findings);
    assert_eq!(
        gaps[0].refs,
        vec!["FR-002".to_string(), "FR-003".to_string()]
    );
    assert_eq!(
        gaps[1].refs,
        vec!["FR-006".to_string(), "FR-007".to_string()]
    );
    assert_eq!(gaps[0].severity, LintSeverity::Major);
}

#[test]
fn numbering_is_tracked_per_prefix_across_sections() {
    let report = lint(
        &input(vec![
            section("spec", "- **FR-001**: one\n- **AS-001**: given\n"),
            section("tasks", "- [ ] T-001: do\n- [x] T-003: done\n"),
        ]),
        now(),
    );
    let gaps = report.by_code(LintCode::NumberingGap);
    assert_eq!(gaps.len(), 1, "findings: {:?}", report.findings);
    assert_eq!(gaps[0].refs, vec!["T-002".to_string()]);
    assert_eq!(gaps[0].section, "tasks");
}

#[test]
fn a_longer_prefix_is_not_mistaken_for_a_tracked_one() {
    let report = lint(
        &input(vec![section(
            "spec",
            "- **NFR-001**: one\n- **NFR-009**: nine\n",
        )]),
        now(),
    );
    assert!(
        report.is_clean(),
        "unexpected findings: {:?}",
        report.findings
    );
}

#[test]
fn a_prose_mention_is_not_a_definition() {
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one, which relates to FR-007 and FR-009\n- **FR-002**: two\n",
        )]),
        now(),
    );
    assert!(
        report.is_clean(),
        "unexpected findings: {:?}",
        report.findings
    );
}

#[test]
fn a_traceability_list_row_is_not_a_second_definition() {
    // SPEC #3248 writes `- FR-191:T-313 / FR-192:T-314` rows in its tasks
    // section. Counting those as definitions reported every mapped
    // requirement as a duplicate.
    let report = lint(
        &input(vec![
            section("spec", "- **FR-001**: one\n- **FR-002**: two\n"),
            section(
                "tasks",
                "- FR-001:T-001 / FR-002:T-002\n- FR-002 coverage note: T-003.\n",
            ),
        ]),
        now(),
    );
    assert!(
        report.by_code(LintCode::NumberingDuplicate).is_empty(),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn a_bold_span_carrying_prose_is_not_a_definition() {
    let report = lint(
        &input(vec![
            section("spec", "- **FR-001**: one\n- **FR-002**: two\n"),
            section(
                "tasks",
                "- **FR-002 classifier misfire (fixed)**: see T-239.\n",
            ),
        ]),
        now(),
    );
    assert!(
        report.by_code(LintCode::NumberingDuplicate).is_empty(),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn a_scoped_composite_reference_is_not_a_tracked_number() {
    // `FR-4237-001` is a per-Issue namespace, not FR-4237; reading it as one
    // stretched the FR range to 4237 and manufactured a gap finding for every
    // number in between.
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one\n- **FR-002**: two\n- FR-4237-001: scoped\n",
        )]),
        now(),
    );
    assert!(
        report.is_clean(),
        "unexpected findings: {:?}",
        report.findings
    );
}

#[test]
fn headings_and_task_checkboxes_still_define() {
    let report = lint(
        &input(vec![
            section("spec", "#### FR-001 Launch\n\n#### FR-003 Stop\n"),
            section("tasks", "- [x] T-001: done\n- [ ] T-003: open\n"),
        ]),
        now(),
    );
    let gaps = report.by_code(LintCode::NumberingGap);
    assert_eq!(gaps.len(), 2, "findings: {:?}", report.findings);
    assert_eq!(gaps[0].refs, vec!["FR-002".to_string()]);
    assert_eq!(gaps[1].refs, vec!["T-002".to_string()]);
}

#[test]
fn a_heading_naming_two_references_defines_neither() {
    // SPEC #3248 carries `### FR-036 / FR-053 条件`, a condition that applies
    // to two requirements defined elsewhere.
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one\n- **FR-002**: two\n\n### FR-001 / FR-002 conditions\n",
        )]),
        now(),
    );
    assert!(
        report.is_clean(),
        "unexpected findings: {:?}",
        report.findings
    );
}

// --- AC-2: three independent classes -----------------------------------

#[test]
fn traceability_row_without_definition_is_its_own_finding() {
    let report = lint(
        &input(vec![
            section("spec", "- **FR-001**: one\n"),
            section(
                "plan",
                "| Requirement | Phase |\n| --- | --- |\n| FR-001 | P0 |\n| FR-002 | P1 |\n",
            ),
        ]),
        now(),
    );
    let orphan_rows = report.by_code(LintCode::TraceabilityRowWithoutDefinition);
    assert_eq!(orphan_rows.len(), 1, "findings: {:?}", report.findings);
    assert_eq!(orphan_rows[0].refs, vec!["FR-002".to_string()]);
    assert_eq!(orphan_rows[0].severity, LintSeverity::Critical);
}

#[test]
fn definition_without_traceability_row_is_its_own_finding() {
    let report = lint(
        &input(vec![
            section("spec", "- **FR-001**: one\n- **FR-002**: two\n"),
            section(
                "plan",
                "| Requirement | Task |\n| --- | --- |\n| FR-001 | ok |\n",
            ),
        ]),
        now(),
    );
    let untracked = report.by_code(LintCode::DefinitionWithoutTraceabilityRow);
    assert_eq!(untracked.len(), 1, "findings: {:?}", report.findings);
    assert_eq!(untracked[0].refs, vec!["FR-002".to_string()]);
    assert_eq!(untracked[0].severity, LintSeverity::Major);
}

#[test]
fn a_prefix_absent_from_the_table_is_not_flagged_as_untracked() {
    let report = lint(
        &input(vec![
            section("spec", "- **FR-001**: one\n- **AS-001**: given\n"),
            section(
                "plan",
                "| Requirement | Task |\n| --- | --- |\n| FR-001 | ok |\n",
            ),
        ]),
        now(),
    );
    assert!(
        report
            .by_code(LintCode::DefinitionWithoutTraceabilityRow)
            .is_empty(),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn each_traceability_direction_aggregates_into_one_finding() {
    // A partial matrix on a large SPEC must not bury every other class under
    // one row per requirement.
    let spec: String = (1..=30)
        .map(|n| format!("- **FR-{n:03}**: requirement {n}\n"))
        .collect();
    let report = lint(
        &input(vec![
            section("spec", &spec),
            section(
                "plan",
                "| Requirement | Phase |\n| --- | --- |\n| FR-001 | P0 |\n| FR-900 | P1 |\n",
            ),
        ]),
        now(),
    );

    let untracked = report.by_code(LintCode::DefinitionWithoutTraceabilityRow);
    assert_eq!(untracked.len(), 1, "findings: {:?}", report.findings);
    assert_eq!(untracked[0].refs.len(), 29);
    assert!(
        untracked[0].message.contains("(+17 more)"),
        "message: {}",
        untracked[0].message
    );

    let orphans = report.by_code(LintCode::TraceabilityRowWithoutDefinition);
    assert_eq!(orphans.len(), 1, "findings: {:?}", report.findings);
    assert_eq!(orphans[0].refs, vec!["FR-900".to_string()]);
}

#[test]
fn missing_supersede_annotation_is_its_own_finding() {
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one\n\
             - **FR-002**: two\n\
             \n\
             ## 2026-07-09 Amendment\n\
             \n\
             This amendment supersedes FR-002.\n",
        )]),
        now(),
    );
    let missing = report.by_code(LintCode::SupersedeAnnotationMissing);
    assert_eq!(missing.len(), 1, "findings: {:?}", report.findings);
    assert_eq!(missing[0].refs, vec!["FR-002".to_string()]);
    assert_eq!(missing[0].severity, LintSeverity::Critical);
}

#[test]
fn an_inline_supersede_note_on_the_definition_clears_the_finding() {
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one\n\
             - **FR-002**: two (superseded by the 2026-07-09 Amendment)\n\
             \n\
             ## 2026-07-09 Amendment\n\
             \n\
             This amendment supersedes FR-002.\n",
        )]),
        now(),
    );
    assert!(
        report
            .by_code(LintCode::SupersedeAnnotationMissing)
            .is_empty(),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn broken_section_marker_is_its_own_finding() {
    let mut artifact = input(vec![section("spec", "- **FR-001**: one\n")]);
    artifact.marker_header = Some(
        "<!-- gwt-spec id=4541 version=1 -->\n<!-- sections:\nspec=body\ntasks=body\n-->\n"
            .to_string(),
    );
    let report = lint(&artifact, now());
    let marker = report.by_code(LintCode::SectionMarkerBroken);
    assert_eq!(marker.len(), 1, "findings: {:?}", report.findings);
    assert!(
        marker[0].message.contains("tasks"),
        "message: {}",
        marker[0].message
    );
    assert_eq!(marker[0].severity, LintSeverity::Critical);
}

#[test]
fn an_unterminated_sections_marker_is_reported() {
    let mut artifact = input(vec![section("spec", "- **FR-001**: one\n")]);
    artifact.marker_header =
        Some("<!-- gwt-spec id=4541 version=1 -->\n<!-- sections:\nspec=body\n".to_string());
    let report = lint(&artifact, now());
    let marker = report.by_code(LintCode::SectionMarkerBroken);
    assert_eq!(marker.len(), 1, "findings: {:?}", report.findings);
    assert!(
        marker[0].message.contains("unterminated"),
        "message: {}",
        marker[0].message
    );
}

#[test]
fn a_roundtrip_hash_mismatch_is_its_own_finding() {
    let mut artifact = input(vec![section("spec", "- **FR-001**: one\n")]);
    artifact.roundtrip = vec![RoundtripObservation {
        section: "spec".to_string(),
        written_sha256: "aaa".to_string(),
        readback_sha256: "bbb".to_string(),
    }];
    let report = lint(&artifact, now());
    let mismatch = report.by_code(LintCode::RoundtripMismatch);
    assert_eq!(mismatch.len(), 1, "findings: {:?}", report.findings);
    assert_eq!(mismatch[0].severity, LintSeverity::Critical);
}

#[test]
fn the_three_ac2_classes_stay_separate_findings() {
    let mut artifact = input(vec![
        section(
            "spec",
            "- **FR-001**: one\n\
             - **FR-002**: two\n\
             \n\
             ## 2026-07-09 Amendment\n\
             \n\
             This amendment supersedes FR-002.\n",
        ),
        section(
            "plan",
            "| Requirement | Task |\n| --- | --- |\n| FR-001 | ok |\n| FR-009 | missing |\n",
        ),
    ]);
    artifact.marker_header =
        Some("<!-- gwt-spec id=4541 version=1 -->\n<!-- sections:\nspec=body\n-->\n".to_string());
    artifact.roundtrip = vec![RoundtripObservation {
        section: "spec".to_string(),
        written_sha256: "aaa".to_string(),
        readback_sha256: "bbb".to_string(),
    }];

    let report = lint(&artifact, now());
    let observed = codes(&report);
    for expected in [
        LintCode::TraceabilityRowWithoutDefinition,
        LintCode::DefinitionWithoutTraceabilityRow,
        LintCode::SupersedeAnnotationMissing,
        LintCode::SectionMarkerBroken,
        LintCode::RoundtripMismatch,
    ] {
        assert!(
            observed.contains(&expected),
            "{expected:?} missing from {observed:?}"
        );
    }
    // Each class produced its own finding — none were merged.
    assert_eq!(
        report.findings.len(),
        observed.len(),
        "findings: {:?}",
        report.findings
    );
}

#[test]
fn finding_ids_are_stable_and_sequential() {
    let report = lint(
        &input(vec![section(
            "spec",
            "- **FR-001**: one\n- **FR-001**: one again\n- **FR-004**: four\n",
        )]),
        now(),
    );
    let ids: Vec<&str> = report
        .findings
        .iter()
        .map(|finding| finding.id.as_str())
        .collect();
    assert_eq!(ids, vec!["L-001", "L-002"]);
    assert_eq!(report.critical_count(), 1);
}

#[test]
fn a_report_round_trips_through_json() {
    let report = lint(
        &input(vec![section("spec", "- **FR-001**: a\n- **FR-001**: b\n")]),
        now(),
    );
    let encoded = serde_json::to_string(&report).unwrap();
    let decoded: LintReport = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, report);
    assert!(encoded.contains("numbering_duplicate"));
}
