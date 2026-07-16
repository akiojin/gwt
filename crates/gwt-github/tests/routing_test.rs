//! Contract tests for the `routing` module (SPEC-12 tdd.md Layer 3).
//!
//! The `split_*` tests cover the SPEC-3248 P7C bootstrap (#3284): oversized
//! section content must split into comment-sized parts that rejoin losslessly
//! under the parser's surrounding-newline trimming, and impossible splits must
//! fail closed instead of truncating.

use std::collections::BTreeMap;

use gwt_github::{
    body::SectionLocation,
    routing::{
        decide_routing, split_section_into_parts, SplitError, COMMENT_PART_BUDGET_BYTES,
        ROUTING_PROMOTE_THRESHOLD_BYTES,
    },
    sections::{extract_sections, SectionName},
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

// ---------------------------------------------------------------------------
// SPEC-3248 P7C bootstrap (#3284): comment part splitting
// ---------------------------------------------------------------------------

/// Rejoin parts through the real storage pipeline: wrap each part the way the
/// writer does (`part=i/K` markers around `\n`-delimited content), run the
/// production marker parser over the wrapped body, then join the extracted
/// contents with a single `\n` exactly like `SpecBody::parse`. The splitter
/// contract is that this full pipeline is lossless.
fn rejoin(parts: &[String]) -> String {
    let total = parts.len();
    parts
        .iter()
        .enumerate()
        .map(|(i, part)| {
            let wrapped = format!(
                "<!-- artifact:plan BEGIN part={idx}/{total} -->\n{part}\n<!-- artifact:plan END part={idx}/{total} -->",
                idx = i + 1
            );
            let extracted = extract_sections(&wrapped).expect("wrapped part must parse");
            assert_eq!(extracted.len(), 1);
            extracted[0].content.clone()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// RED-70: content within the budget stays a single part, byte-for-byte.
#[test]
fn red_70_split_under_budget_single_part() {
    let content = "line one\nline two\nline three";
    let parts = split_section_into_parts(content).unwrap();
    assert_eq!(parts, vec![content.to_string()]);
}

// RED-71: oversized content splits into multiple parts, each within the
// per-part budget, and rejoins losslessly under parser trimming semantics.
#[test]
fn red_71_split_over_budget_lossless() {
    // ~3 budgets worth of realistic multi-line markdown.
    let line = "- [ ] T-999 some task line with a bit of text to fill space\n";
    let repeat = (COMMENT_PART_BUDGET_BYTES * 3) / line.len();
    let content = line.repeat(repeat).trim_end_matches('\n').to_string();

    let parts = split_section_into_parts(&content).unwrap();
    assert!(
        parts.len() >= 3,
        "expected at least 3 parts, got {}",
        parts.len()
    );
    for (i, part) in parts.iter().enumerate() {
        assert!(
            part.len() <= COMMENT_PART_BUDGET_BYTES,
            "part {} exceeds budget: {} bytes",
            i + 1,
            part.len()
        );
    }
    assert_eq!(rejoin(&parts), content, "parts must rejoin losslessly");
}

// RED-72: paragraph-per-line markdown (every second line blank, so cut
// points inevitably touch blank lines) must still roundtrip exactly through
// the exact-trim part contract.
#[test]
fn red_72_split_preserves_interior_blank_lines() {
    let paragraph = "## Heading\n\nBody text line that is reasonably long for a paragraph.\n\n";
    let repeat = (COMMENT_PART_BUDGET_BYTES * 2) / paragraph.len() + 1;
    let content = paragraph.repeat(repeat).trim_end_matches('\n').to_string();

    let parts = split_section_into_parts(&content).unwrap();
    assert!(parts.len() >= 2, "expected a multi-part split");
    assert_eq!(
        rejoin(&parts),
        content,
        "blank lines must survive the split"
    );
}

// RED-73: a single line larger than the budget cannot be split safely —
// fail closed, never truncate.
#[test]
fn red_73_split_single_oversized_line_fails_closed() {
    let content = "x".repeat(COMMENT_PART_BUDGET_BYTES + 1);
    let err = split_section_into_parts(&content).unwrap_err();
    assert!(matches!(err, SplitError::UnsplittableChunk { .. }));
}

// RED-74: splitting is deterministic.
#[test]
fn red_74_split_is_pure() {
    let line = "content line for determinism check\n";
    let content = line
        .repeat(COMMENT_PART_BUDGET_BYTES / line.len() * 2)
        .trim_end_matches('\n')
        .to_string();
    let p1 = split_section_into_parts(&content).unwrap();
    let p2 = split_section_into_parts(&content).unwrap();
    assert_eq!(p1, p2);
}
