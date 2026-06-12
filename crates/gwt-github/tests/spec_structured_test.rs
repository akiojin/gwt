//! Public API contract tests for `gwt_github::spec_structured` (SPEC-3060).
//!
//! The structured spec schema moved here from the gwtd CLI so that any
//! client can parse / render / merge canonical SPEC sections. These tests
//! pin the externally observable rules: priority normalization, FR / NFR /
//! SC renumbering, canonical-section merge semantics, and unknown-section
//! preservation.

use gwt_github::spec_structured::{
    merge_structured_spec, parse_structured_spec_json, render_structured_spec,
    split_structured_spec, StructuredSpecInput, StructuredUserStory, TextBlock,
};

fn story(title: &str, priority: Option<&str>) -> StructuredUserStory {
    StructuredUserStory {
        title: title.to_string(),
        priority: priority.map(str::to_string),
        status: None,
        statement: None,
        as_a: Some("a developer".to_string()),
        i_want: Some("one schema owner".to_string()),
        so_that: Some("rules change in one place".to_string()),
        acceptance_scenarios: vec!["- Given X, When Y, Then Z".to_string()],
    }
}

#[test]
fn render_normalizes_priority_and_renumbers_requirements() {
    let input = StructuredSpecInput {
        background: Some(TextBlock::Text("  why this exists  ".to_string())),
        user_stories: Some(vec![story("US-9: Reuse the schema", Some("1"))]),
        edge_cases: None,
        functional_requirements: Some(vec![
            "FR-007: keep behavior identical".to_string(),
            "- expose a public module".to_string(),
        ]),
        non_functional_requirements: Some(vec!["NFR-3: no perf regression".to_string()]),
        success_criteria: Some(vec!["**SC-002**: all suites stay green".to_string()]),
    };

    let rendered = render_structured_spec("Move structured schema", &input);

    assert!(rendered.starts_with("# Move structured schema\n"));
    assert!(rendered.contains("## Background\n\nwhy this exists"));
    // US prefix in the input title is stripped and the story is renumbered.
    assert!(rendered.contains("### US-1: Reuse the schema (P1)"));
    assert!(rendered
        .contains("As a developer, I want one schema owner, so that rules change in one place."));
    assert!(rendered.contains("1. Given X, When Y, Then Z"));
    // Requirement labels are stripped and renumbered contiguously.
    assert!(rendered.contains("- **FR-001**: keep behavior identical"));
    assert!(rendered.contains("- **FR-002**: expose a public module"));
    assert!(rendered.contains("- **NFR-001**: no perf regression"));
    assert!(rendered.contains("- **SC-001**: all suites stay green"));
}

#[test]
fn render_returns_title_heading_only_when_all_sections_are_empty() {
    let rendered = render_structured_spec("Bare Title", &StructuredSpecInput::default());
    assert_eq!(rendered, "# Bare Title\n");
}

#[test]
fn merge_replaces_canonical_sections_and_preserves_unknown_sections() {
    let existing =
        "# Title\n\n## Background\n\nold background\n\n## Implementation Notes\n\nkeep me\n";
    let patch = StructuredSpecInput {
        background: Some(TextBlock::Text("new background".to_string())),
        ..StructuredSpecInput::default()
    };

    let merged = merge_structured_spec(existing, &patch);

    assert!(merged.contains("## Background\n\nnew background"));
    assert!(!merged.contains("old background"));
    assert!(
        merged.contains("## Implementation Notes\n\nkeep me"),
        "unknown sections must survive a merge: {merged}"
    );
}

#[test]
fn merge_removes_canonical_section_when_patch_renders_empty() {
    let existing = "# Title\n\n## Edge Cases\n\n- old edge\n";
    let patch = StructuredSpecInput {
        edge_cases: Some(vec![]),
        ..StructuredSpecInput::default()
    };

    let merged = merge_structured_spec(existing, &patch);

    assert!(
        !merged.contains("## Edge Cases"),
        "an explicitly emptied section must be removed: {merged}"
    );
}

#[test]
fn split_extracts_title_and_separates_unknown_sections() {
    let existing = "# My Spec\n\n## Background\n\ncontext\n\n## Rollout Plan\n\nphase 1\n";
    let (title, known, unknown) = split_structured_spec(existing);

    assert_eq!(title, "My Spec");
    assert!(known.contains_key("Background"));
    assert_eq!(unknown.len(), 1);
    assert!(unknown[0].contains("## Rollout Plan"));
}

#[test]
fn parse_rejects_invalid_json_with_spec_ops_error() {
    let error = parse_structured_spec_json("{not json").expect_err("invalid json must fail");
    assert!(
        error.to_string().contains("invalid spec json"),
        "unexpected error: {error}"
    );
}
