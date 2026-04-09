//! Contract tests for the `cache` module (SPEC-12 tdd.md Layer 6).

use std::collections::BTreeMap;
use std::fs;

use gwt_github::body::{Comment, SectionLocation, SectionsIndex, SpecBody, SpecMeta};
use gwt_github::cache::{Cache, CacheEntry};
use gwt_github::client::{
    CommentId, CommentSnapshot, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
};
use gwt_github::sections::SectionName;
use tempfile::TempDir;

fn n(s: &str) -> SectionName {
    SectionName(s.to_string())
}

fn mk_body_with_spec_and_tasks_in_body(spec: &str, tasks: &str) -> String {
    format!(
        "<!-- gwt-spec id=42 version=1 -->\n\
<!-- sections:\n\
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
<!-- artifact:tasks END -->\n"
    )
}

fn mk_snapshot(number: u64, body: String) -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(number),
        title: format!("Spec {}", number),
        body,
        labels: vec!["gwt-spec".to_string(), "phase/review".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t1"),
        comments: Vec::new(),
    }
}

// RED-50: write_snapshot creates per-issue directory and files
#[test]
fn red_50_write_snapshot_creates_directory_layout() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body = mk_body_with_spec_and_tasks_in_body("SPEC BODY", "- [ ] T-001");
    let snapshot = mk_snapshot(42, body);

    cache.write_snapshot(&snapshot).unwrap();

    let issue_dir = tmp.path().join("42");
    assert!(issue_dir.is_dir(), "issue dir should exist");
    assert!(issue_dir.join("body.md").is_file());
    assert!(issue_dir.join("meta.json").is_file());
    assert!(issue_dir.join("sections/spec.md").is_file());
    assert!(issue_dir.join("sections/tasks.md").is_file());
}

// RED-51: written body.md is byte-identical to the snapshot body
#[test]
fn red_51_body_md_is_verbatim() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body = mk_body_with_spec_and_tasks_in_body("alpha", "beta");
    let snapshot = mk_snapshot(7, body.clone());

    cache.write_snapshot(&snapshot).unwrap();

    let written = fs::read_to_string(tmp.path().join("7/body.md")).unwrap();
    assert_eq!(written, body);
}

// RED-52: sections/*.md files contain the parsed section content (no markers)
#[test]
fn red_52_section_files_contain_parsed_content_only() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body = mk_body_with_spec_and_tasks_in_body("spec body", "tasks body");
    let snapshot = mk_snapshot(3, body);

    cache.write_snapshot(&snapshot).unwrap();

    let spec_txt = fs::read_to_string(tmp.path().join("3/sections/spec.md")).unwrap();
    let tasks_txt = fs::read_to_string(tmp.path().join("3/sections/tasks.md")).unwrap();
    assert_eq!(spec_txt, "spec body");
    assert_eq!(tasks_txt, "tasks body");
}

// RED-53: meta.json round-trips updated_at / number / labels / state / title
#[test]
fn red_53_meta_json_round_trips() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body = mk_body_with_spec_and_tasks_in_body("a", "b");
    let mut snapshot = mk_snapshot(99, body);
    snapshot.updated_at = UpdatedAt::new("2026-04-08T12:34:56Z");
    snapshot.state = IssueState::Closed;

    cache.write_snapshot(&snapshot).unwrap();

    let entry = cache.load_entry(IssueNumber(99)).unwrap();
    assert_eq!(entry.snapshot.number, IssueNumber(99));
    assert_eq!(
        entry.snapshot.updated_at,
        UpdatedAt::new("2026-04-08T12:34:56Z")
    );
    assert_eq!(entry.snapshot.state, IssueState::Closed);
    assert_eq!(entry.snapshot.labels, vec!["gwt-spec", "phase/review"]);
}

// RED-54: read_section returns the current cache for a body-resident section
#[test]
fn red_54_read_section_from_body() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body = mk_body_with_spec_and_tasks_in_body("spec content", "tasks content");
    cache.write_snapshot(&mk_snapshot(10, body)).unwrap();

    let content = cache
        .read_section(IssueNumber(10), &n("spec"))
        .unwrap()
        .unwrap();
    assert_eq!(content, "spec content");
}

// RED-55: load_entry for an unknown issue returns None
#[test]
fn red_55_load_unknown_issue_returns_none() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    assert!(cache.load_entry(IssueNumber(9999)).is_none());
}

// RED-56: subsequent writes replace the prior body.md atomically (no stray tmp files)
#[test]
fn red_56_subsequent_write_replaces_body() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body_v1 = mk_body_with_spec_and_tasks_in_body("v1", "t1");
    let body_v2 = mk_body_with_spec_and_tasks_in_body("v2", "t2");
    cache.write_snapshot(&mk_snapshot(1, body_v1)).unwrap();
    cache
        .write_snapshot(&mk_snapshot(1, body_v2.clone()))
        .unwrap();

    let written = fs::read_to_string(tmp.path().join("1/body.md")).unwrap();
    assert_eq!(written, body_v2);
    // No stray .tmp files left behind.
    for entry in fs::read_dir(tmp.path().join("1")).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        assert!(!name.ends_with(".tmp"), "stray tmp file found: {}", name);
    }
}

// RED-57: write_snapshot with body containing comments also writes comments/*.md
#[test]
fn red_57_write_snapshot_with_comments() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body = "<!-- gwt-spec id=5 version=1 -->\n\
<!-- sections:\n\
plan=comment:100\n\
spec=body\n\
tasks=body\n\
-->\n\
\n\
<!-- artifact:spec BEGIN -->\n\
s\n\
<!-- artifact:spec END -->\n\
\n\
<!-- artifact:tasks BEGIN -->\n\
t\n\
<!-- artifact:tasks END -->\n"
        .to_string();
    let mut snapshot = mk_snapshot(5, body);
    snapshot.comments.push(CommentSnapshot {
        id: CommentId(100),
        body: "<!-- artifact:plan BEGIN -->\nplan body\n<!-- artifact:plan END -->".to_string(),
        updated_at: UpdatedAt::new("t2"),
    });

    cache.write_snapshot(&snapshot).unwrap();

    let comment_path = tmp.path().join("5/comments/100.md");
    assert!(comment_path.is_file(), "comment file should exist");
    let plan_section = fs::read_to_string(tmp.path().join("5/sections/plan.md")).unwrap();
    assert_eq!(plan_section, "plan body");
}

// RED-58: load_entry reconstructs a SpecBody view consistent with the snapshot
#[test]
fn red_58_load_entry_reconstructs_spec_body() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body = mk_body_with_spec_and_tasks_in_body("s", "t");
    cache.write_snapshot(&mk_snapshot(11, body)).unwrap();

    let entry: CacheEntry = cache.load_entry(IssueNumber(11)).unwrap();
    let spec_body: SpecBody = entry.spec_body.clone();
    assert_eq!(
        spec_body.meta,
        SpecMeta {
            id: "42".to_string(),
            version: 1
        }
    );
    let expected_index: BTreeMap<SectionName, SectionLocation> = [
        (n("spec"), SectionLocation::Body),
        (n("tasks"), SectionLocation::Body),
    ]
    .into_iter()
    .collect();
    assert_eq!(spec_body.sections_index, SectionsIndex(expected_index));
    assert_eq!(
        spec_body.sections.get(&n("spec")).cloned(),
        Some("s".to_string())
    );
    assert_eq!(
        spec_body.sections.get(&n("tasks")).cloned(),
        Some("t".to_string())
    );
}

// RED-59: read_section for a section that does not exist returns Ok(None)
#[test]
fn red_59_read_missing_section_returns_none() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let body = mk_body_with_spec_and_tasks_in_body("s", "t");
    cache.write_snapshot(&mk_snapshot(1, body)).unwrap();
    let got = cache.read_section(IssueNumber(1), &n("plan")).unwrap();
    assert!(got.is_none());
}

// Regression (CodeRabbit / PR #1943): write_snapshot must sweep
// section files that belonged to a prior snapshot but are absent from
// the current one. Without this, `read_section` returns stale content
// for a section the Issue has already deleted.
#[test]
fn write_snapshot_prunes_stale_section_files() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());

    // v1: spec + tasks sections in the body.
    let body_v1 = mk_body_with_spec_and_tasks_in_body("s1", "t1");
    cache.write_snapshot(&mk_snapshot(42, body_v1)).unwrap();

    let sections_dir = tmp.path().join("42").join("sections");
    assert!(
        sections_dir.join("spec.md").exists(),
        "v1 should have spec.md"
    );
    assert!(
        sections_dir.join("tasks.md").exists(),
        "v1 should have tasks.md"
    );

    // v2: only the `spec` section remains (tasks was removed).
    let body_v2 = "<!-- gwt-spec id=42 version=1 -->\n\
<!-- sections:\n\
spec=body\n\
-->\n\
\n\
<!-- artifact:spec BEGIN -->\n\
s2\n\
<!-- artifact:spec END -->\n"
        .to_string();
    cache.write_snapshot(&mk_snapshot(42, body_v2)).unwrap();

    assert!(
        sections_dir.join("spec.md").exists(),
        "v2 spec.md should still exist"
    );
    assert!(
        !sections_dir.join("tasks.md").exists(),
        "v2 should have pruned the stale tasks.md, but it is still present"
    );

    // Reading the pruned section must return None, not the stale v1
    // content.
    assert!(cache
        .read_section(IssueNumber(42), &n("tasks"))
        .unwrap()
        .is_none());
}

// Companion regression: stale comment files must also be swept when a
// snapshot drops a comment id.
#[test]
fn write_snapshot_prunes_stale_comment_files() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());

    let mk = |comment_ids: &[u64]| IssueSnapshot {
        number: IssueNumber(7),
        title: "Spec 7".into(),
        body: mk_body_with_spec_and_tasks_in_body("s", "t"),
        labels: vec!["gwt-spec".into()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t1"),
        comments: comment_ids
            .iter()
            .map(|id| CommentSnapshot {
                id: CommentId(*id),
                body: format!("comment {id}"),
                updated_at: UpdatedAt::new("t1"),
            })
            .collect(),
    };

    cache.write_snapshot(&mk(&[100, 200, 300])).unwrap();
    let comments_dir = tmp.path().join("7").join("comments");
    assert!(comments_dir.join("100.md").exists());
    assert!(comments_dir.join("200.md").exists());
    assert!(comments_dir.join("300.md").exists());

    cache.write_snapshot(&mk(&[100, 300])).unwrap();
    assert!(comments_dir.join("100.md").exists());
    assert!(
        !comments_dir.join("200.md").exists(),
        "v2 must prune the dropped 200.md"
    );
    assert!(comments_dir.join("300.md").exists());
}

// Keep Comment in use so the import is not a lint warning.
#[allow(dead_code)]
fn _ensure_comment_is_used(c: Comment) -> Comment {
    c
}
