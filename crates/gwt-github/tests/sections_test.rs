//! Contract tests for the `sections` module (SPEC-12 tdd.md Layer 1).

use gwt_github::sections::{
    extract_sections, ExtractedSection, SectionName, SectionParseError, SectionPart,
};

fn name(s: &str) -> SectionName {
    SectionName(s.to_string())
}

// RED-01: single section extraction
#[test]
fn red_01_single_section() {
    let input = "<!-- artifact:spec BEGIN -->\nfoo\n<!-- artifact:spec END -->";
    let got = extract_sections(input).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].name, name("spec"));
    assert_eq!(got[0].content, "foo");
    assert!(got[0].part.is_none());
}

// RED-02: multiple sections in order
#[test]
fn red_02_multiple_sections_preserves_order() {
    let input = "\
<!-- artifact:spec BEGIN -->
spec-body
<!-- artifact:spec END -->

<!-- artifact:tasks BEGIN -->
tasks-body
<!-- artifact:tasks END -->

<!-- artifact:plan BEGIN -->
plan-body
<!-- artifact:plan END -->
";
    let got = extract_sections(input).unwrap();
    let names: Vec<_> = got.iter().map(|s| s.name.0.as_str()).collect();
    assert_eq!(names, vec!["spec", "tasks", "plan"]);
    assert_eq!(got[0].content, "spec-body");
    assert_eq!(got[1].content, "tasks-body");
    assert_eq!(got[2].content, "plan-body");
}

// RED-03: BEGIN without END
#[test]
fn red_03_unterminated_section() {
    let input = "<!-- artifact:spec BEGIN -->\nfoo without end";
    let err = extract_sections(input).unwrap_err();
    assert!(matches!(err, SectionParseError::UnterminatedSection(ref s) if s == "spec"));
}

// RED-04: END without BEGIN
#[test]
fn red_04_unmatched_end() {
    let input = "stray content <!-- artifact:spec END -->";
    let err = extract_sections(input).unwrap_err();
    assert!(matches!(err, SectionParseError::UnmatchedEnd(ref s) if s == "spec"));
}

// RED-05: duplicate section name
#[test]
fn red_05_duplicate_section() {
    let input = "\
<!-- artifact:spec BEGIN -->
first
<!-- artifact:spec END -->
<!-- artifact:spec BEGIN -->
second
<!-- artifact:spec END -->
";
    let err = extract_sections(input).unwrap_err();
    assert!(matches!(err, SectionParseError::DuplicateSection(ref s) if s == "spec"));
}

// RED-06: section name with slash (contract/checklist paths)
#[test]
fn red_06_slashed_section_name() {
    let input = "\
<!-- artifact:contract/api.yaml BEGIN -->
openapi: 3.1.0
<!-- artifact:contract/api.yaml END -->
<!-- artifact:checklist/tdd.md BEGIN -->
- [ ] cover X
<!-- artifact:checklist/tdd.md END -->
";
    let got = extract_sections(input).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, name("contract/api.yaml"));
    assert_eq!(got[0].content, "openapi: 3.1.0");
    assert_eq!(got[1].name, name("checklist/tdd.md"));
    assert_eq!(got[1].content, "- [ ] cover X");
}

// RED-07: leading/trailing whitespace inside section body is trimmed of surrounding
// newlines only (internal indentation preserved).
#[test]
fn red_07_strips_surrounding_newlines_only() {
    let input = "\
<!-- artifact:spec BEGIN -->


  indented content
  with two spaces

<!-- artifact:spec END -->
";
    let got = extract_sections(input).unwrap();
    assert_eq!(got[0].content, "  indented content\n  with two spaces");
}

// RED-08: split section with part=1/2, part=2/2 markers
#[test]
fn red_08_split_section_parts() {
    let input = "\
<!-- artifact:plan BEGIN part=1/2 -->
first half
<!-- artifact:plan END part=1/2 -->

<!-- artifact:plan BEGIN part=2/2 -->
second half
<!-- artifact:plan END part=2/2 -->
";
    let got = extract_sections(input).unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, name("plan"));
    assert_eq!(got[0].part, Some(SectionPart { index: 1, total: 2 }));
    assert_eq!(got[0].content, "first half");
    assert_eq!(got[1].name, name("plan"));
    assert_eq!(got[1].part, Some(SectionPart { index: 2, total: 2 }));
    assert_eq!(got[1].content, "second half");
}

// RED-09: mismatched BEGIN/END part suffixes are an error
#[test]
fn red_09_mismatched_part_suffix() {
    let input = "\
<!-- artifact:plan BEGIN part=1/2 -->
mismatched
<!-- artifact:plan END -->
";
    let err = extract_sections(input).unwrap_err();
    // Should surface as malformed marker since BEGIN specified part but END did not.
    assert!(matches!(err, SectionParseError::MalformedMarker(_)));
}

// RED-10: empty section body is allowed
#[test]
fn red_10_empty_section_body() {
    let input = "<!-- artifact:plan BEGIN -->\n<!-- artifact:plan END -->";
    let got = extract_sections(input).unwrap();
    assert_eq!(got[0].content, "");
}

// Helper: construct an ExtractedSection for assertions in other layers.
#[allow(dead_code)]
fn mk(name_str: &str, content: &str) -> ExtractedSection {
    ExtractedSection {
        name: name(name_str),
        content: content.to_string(),
        part: None,
    }
}
