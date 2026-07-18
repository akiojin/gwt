//! Contract tests for the `body` module (SPEC-12 tdd.md Layer 2).

use gwt_github::{
    body::{Comment, ParseError, SectionLocation, SectionsIndex, SpecBody, SpecMeta},
    sections::SectionName,
};

fn n(s: &str) -> SectionName {
    SectionName(s.to_string())
}

fn mk_body(spec_content: &str, tasks_content: &str, plan_comment_id: u64) -> String {
    format!(
        "<!-- gwt-spec id=2001 version=1 -->\n\
<!-- sections:\n\
plan=comment:{plan}\n\
spec=body\n\
tasks=body\n\
-->\n\
\n\
<!-- artifact:spec BEGIN -->\n\
{spec}\n\
<!-- artifact:spec END -->\n\
\n\
<!-- artifact:tasks BEGIN -->\n\
{tasks}\n\
<!-- artifact:tasks END -->\n\
",
        plan = plan_comment_id,
        spec = spec_content,
        tasks = tasks_content,
    )
}

fn mk_comment(id: u64, name: &str, content: &str) -> Comment {
    Comment {
        id,
        body: format!("<!-- artifact:{name} BEGIN -->\n{content}\n<!-- artifact:{name} END -->"),
    }
}

// RED-10: parse meta header
#[test]
fn red_10_parse_meta_header() {
    let body = mk_body("spec content", "- [ ] T-001 example", 42);
    let comments = [mk_comment(42, "plan", "plan content")];
    let spec_body = SpecBody::parse(&body, &comments).unwrap();
    assert_eq!(
        spec_body.meta,
        SpecMeta {
            id: "2001".to_string(),
            version: 1
        }
    );
}

// RED-11: parse index map
#[test]
fn red_11_parse_index_map() {
    let body = mk_body("spec content", "tasks content", 42);
    let comments = [mk_comment(42, "plan", "plan content")];
    let spec_body = SpecBody::parse(&body, &comments).unwrap();
    assert_eq!(
        spec_body.sections_index.0.get(&n("spec")),
        Some(&SectionLocation::Body)
    );
    assert_eq!(
        spec_body.sections_index.0.get(&n("tasks")),
        Some(&SectionLocation::Body)
    );
    assert_eq!(
        spec_body.sections_index.0.get(&n("plan")),
        Some(&SectionLocation::Comments(vec![42]))
    );
}

// RED-12: body + comments assemble all sections
#[test]
fn red_12_assemble_body_and_comments() {
    let body = mk_body("spec content", "tasks content", 42);
    let comments = [mk_comment(42, "plan", "plan content\nline 2")];
    let spec_body = SpecBody::parse(&body, &comments).unwrap();
    assert_eq!(
        spec_body
            .sections
            .get(&n("spec"))
            .map(std::string::String::as_str),
        Some("spec content")
    );
    assert_eq!(
        spec_body
            .sections
            .get(&n("tasks"))
            .map(std::string::String::as_str),
        Some("tasks content")
    );
    assert_eq!(
        spec_body
            .sections
            .get(&n("plan"))
            .map(std::string::String::as_str),
        Some("plan content\nline 2")
    );
}

// RED-13: split comments with part=1/2 / part=2/2 are concatenated in order
#[test]
fn red_13_split_comments_concatenated() {
    let body = format!(
        "<!-- gwt-spec id=2002 version=1 -->\n\
<!-- sections:\n\
plan=comment:{c1},comment:{c2}\n\
spec=body\n\
-->\n\
\n\
<!-- artifact:spec BEGIN -->\n\
s\n\
<!-- artifact:spec END -->\n\
",
        c1 = 111,
        c2 = 222
    );
    let comments = [
        Comment {
            id: 111,
            body:
                "<!-- artifact:plan BEGIN part=1/2 -->\nalpha\n<!-- artifact:plan END part=1/2 -->"
                    .to_string(),
        },
        Comment {
            id: 222,
            body:
                "<!-- artifact:plan BEGIN part=2/2 -->\nbeta\n<!-- artifact:plan END part=2/2 -->"
                    .to_string(),
        },
    ];
    let spec_body = SpecBody::parse(&body, &comments).unwrap();
    assert_eq!(
        spec_body
            .sections
            .get(&n("plan"))
            .map(std::string::String::as_str),
        Some("alpha\nbeta")
    );
}

// RED-14: missing comment referenced by index map is an error
#[test]
fn red_14_missing_comment_reference() {
    let body = mk_body("spec content", "tasks content", 999);
    let comments: [Comment; 0] = [];
    let err = SpecBody::parse(&body, &comments).unwrap_err();
    assert!(matches!(
        err,
        ParseError::MissingComment { section, comment_id: 999 } if section == "plan"
    ));
}

// RED-15: missing header is an error
#[test]
fn red_15_missing_header() {
    let body = "no header here\n<!-- sections:\nspec=body\n-->\n<!-- artifact:spec BEGIN -->\nhi\n<!-- artifact:spec END -->";
    let err = SpecBody::parse(body, &[]).unwrap_err();
    assert!(matches!(err, ParseError::MissingHeader));
}

// RED-16: missing index map is an error
#[test]
fn red_16_missing_index_map() {
    let body = "<!-- gwt-spec id=1 version=1 -->\n<!-- artifact:spec BEGIN -->\nhi\n<!-- artifact:spec END -->";
    let err = SpecBody::parse(body, &[]).unwrap_err();
    assert!(matches!(err, ParseError::MissingIndex));
}

// RED-17: splice in place — other sections unchanged
#[test]
fn red_17_splice_preserves_other_sections() {
    let body = mk_body("spec original", "tasks original", 42);
    let comments = [mk_comment(42, "plan", "plan original")];
    let mut spec_body = SpecBody::parse(&body, &comments).unwrap();
    let before_spec = spec_body.sections.get(&n("spec")).cloned();
    let before_plan = spec_body.sections.get(&n("plan")).cloned();

    spec_body.splice(n("tasks"), "tasks REPLACED".to_string());

    assert_eq!(
        spec_body
            .sections
            .get(&n("tasks"))
            .map(std::string::String::as_str),
        Some("tasks REPLACED")
    );
    assert_eq!(spec_body.sections.get(&n("spec")).cloned(), before_spec);
    assert_eq!(spec_body.sections.get(&n("plan")).cloned(), before_plan);
}

// RED-18: splice adds a new section if absent
#[test]
fn red_18_splice_adds_new_section() {
    let body = mk_body("s", "t", 42);
    let comments = [mk_comment(42, "plan", "p")];
    let mut spec_body = SpecBody::parse(&body, &comments).unwrap();
    assert!(!spec_body.sections.contains_key(&n("research")));
    spec_body.splice(n("research"), "research body".to_string());
    assert_eq!(
        spec_body
            .sections
            .get(&n("research"))
            .map(std::string::String::as_str),
        Some("research body")
    );
}

// Helper: expose SectionsIndex to silence unused import on minimal tests.
#[allow(dead_code)]
fn _touch(idx: &SectionsIndex) -> usize {
    idx.0.len()
}

// ---------------------------------------------------------------------------
// SPEC-3248 P7C bootstrap (#3284): fail-closed split-section validation
// ---------------------------------------------------------------------------

fn mk_split_body(section: &str, comment_ids: &[u64]) -> String {
    let refs = comment_ids
        .iter()
        .map(|id| format!("comment:{id}"))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "<!-- gwt-spec id=2003 version=1 -->\n\
<!-- sections:\n\
{section}={refs}\n\
spec=body\n\
-->\n\
\n\
<!-- artifact:spec BEGIN -->\n\
s\n\
<!-- artifact:spec END -->\n"
    )
}

fn mk_part_comment(id: u64, name: &str, index: u32, total: u32, content: &str) -> Comment {
    Comment {
        id,
        body: format!(
            "<!-- artifact:{name} BEGIN part={index}/{total} -->\n{content}\n<!-- artifact:{name} END part={index}/{total} -->"
        ),
    }
}

// RED-80: a part is missing (totals say 3, only parts 1 and 2 referenced) —
// parse must fail closed instead of silently joining a truncated section.
#[test]
fn red_80_incomplete_parts_rejected() {
    let body = mk_split_body("plan", &[111, 222]);
    let comments = [
        mk_part_comment(111, "plan", 1, 3, "alpha"),
        mk_part_comment(222, "plan", 2, 3, "beta"),
    ];
    let err = SpecBody::parse(&body, &comments).unwrap_err();
    assert!(
        matches!(
            &err,
            ParseError::IncompleteParts { section, expected: 3, found: 2 } if section == "plan"
        ),
        "expected IncompleteParts, got {err:?}"
    );
}

// RED-81: duplicate part index must be rejected, not double-joined.
#[test]
fn red_81_duplicate_part_index_rejected() {
    let body = mk_split_body("plan", &[111, 222]);
    let comments = [
        mk_part_comment(111, "plan", 1, 2, "alpha"),
        mk_part_comment(222, "plan", 1, 2, "alpha again"),
    ];
    let err = SpecBody::parse(&body, &comments).unwrap_err();
    assert!(
        matches!(&err, ParseError::IncompleteParts { section, .. } if section == "plan")
            || matches!(&err, ParseError::BrokenIndex(_)),
        "expected fail-closed error, got {err:?}"
    );
}

// RED-82: inconsistent totals across parts must be rejected.
#[test]
fn red_82_inconsistent_totals_rejected() {
    let body = mk_split_body("plan", &[111, 222]);
    let comments = [
        mk_part_comment(111, "plan", 1, 2, "alpha"),
        mk_part_comment(222, "plan", 2, 3, "beta"),
    ];
    let err = SpecBody::parse(&body, &comments).unwrap_err();
    assert!(
        matches!(&err, ParseError::IncompleteParts { section, .. } if section == "plan")
            || matches!(&err, ParseError::BrokenIndex(_)),
        "expected fail-closed error, got {err:?}"
    );
}

// RED-83: mixing a part-marked comment and an unmarked comment for the same
// section is structurally ambiguous — fail closed.
#[test]
fn red_83_mixed_part_and_unmarked_rejected() {
    let body = mk_split_body("plan", &[111, 222]);
    let comments = [
        mk_comment(111, "plan", "alpha"),
        mk_part_comment(222, "plan", 2, 2, "beta"),
    ];
    let err = SpecBody::parse(&body, &comments).unwrap_err();
    assert!(
        matches!(&err, ParseError::IncompleteParts { section, .. } if section == "plan")
            || matches!(&err, ParseError::BrokenIndex(_)),
        "expected fail-closed error, got {err:?}"
    );
}

// RED-84: complete parts listed out of id order still join by part index.
#[test]
fn red_84_out_of_order_ids_join_by_part_index() {
    let body = mk_split_body("plan", &[222, 111]);
    let comments = [
        mk_part_comment(111, "plan", 1, 2, "alpha"),
        mk_part_comment(222, "plan", 2, 2, "beta"),
    ];
    let spec_body = SpecBody::parse(&body, &comments).unwrap();
    assert_eq!(
        spec_body
            .sections
            .get(&n("plan"))
            .map(std::string::String::as_str),
        Some("alpha\nbeta")
    );
}

// RED-85: part count must also match the number of referenced comments —
// referencing the same complete pair twice is rejected.
#[test]
fn red_85_reference_count_mismatch_rejected() {
    let body = mk_split_body("plan", &[111, 222, 111]);
    let comments = [
        mk_part_comment(111, "plan", 1, 2, "alpha"),
        mk_part_comment(222, "plan", 2, 2, "beta"),
    ];
    let err = SpecBody::parse(&body, &comments).unwrap_err();
    assert!(
        matches!(&err, ParseError::IncompleteParts { section, .. } if section == "plan")
            || matches!(&err, ParseError::BrokenIndex(_)),
        "expected fail-closed error, got {err:?}"
    );
}
