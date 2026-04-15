//! Contract tests for the `spec_ops` module (SPEC-12 tdd.md Layer 7).

use gwt_github::{
    client::{
        fake::FakeIssueClient, IssueClient, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
    },
    sections::SectionName,
    spec_ops::{SpecOps, SpecOpsError},
    Cache,
};
use tempfile::TempDir;

fn n(s: &str) -> SectionName {
    SectionName(s.to_string())
}

fn mk_body(spec: &str, tasks: &str) -> String {
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

fn seed_spec(
    client: &FakeIssueClient,
    cache: &Cache,
    number: u64,
    spec: &str,
    tasks: &str,
) -> IssueSnapshot {
    let body = mk_body(spec, tasks);
    let snapshot = IssueSnapshot {
        number: IssueNumber(number),
        title: format!("SPEC {number}"),
        body,
        labels: vec!["gwt-spec".to_string(), "phase/review".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new(format!("seed-{number}")),
        comments: Vec::new(),
    };
    client.seed(snapshot.clone());
    cache.write_snapshot(&snapshot).unwrap();
    snapshot
}

// RED-60: read_section with fresh cache -> NotModified -> reads from cache
#[test]
fn red_60_read_section_uses_cache_on_not_modified() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "spec content", "tasks content");

    let ops = SpecOps::new(client, cache);
    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, "tasks content");

    // Expect exactly one fetch (conditional, returning NotModified) and zero body patches.
    let log = ops.client().call_log();
    assert!(log.contains(&"fetch:#1".to_string()));
    assert!(!log.iter().any(|l| l.starts_with("patch_body:")));
}

// RED-61: read_section after server mutation -> Updated -> cache overwritten
#[test]
fn red_61_read_section_refreshes_cache_when_updated() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    // Server-side mutation (simulate someone else editing the issue).
    let new_body = mk_body("v2", "t2");
    client.patch_body(IssueNumber(1), &new_body).unwrap();

    let ops = SpecOps::new(client, cache);
    let got = ops.read_section(IssueNumber(1), &n("spec")).unwrap();
    assert_eq!(got, "v2");

    // Cache must now reflect the new content for the other section too.
    let tasks_got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(tasks_got, "t2");
}

// RED-62: write_section body->body produces exactly one patch_body call
#[test]
fn red_62_write_section_body_to_body_single_patch() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    ops.write_section(IssueNumber(1), &n("tasks"), "t2")
        .unwrap();

    let log = ops.client().call_log();
    let patch_body_count = log
        .iter()
        .filter(|l| l.starts_with("patch_body:#1"))
        .count();
    assert_eq!(
        patch_body_count, 1,
        "expected exactly one patch_body call, got {patch_body_count}: {log:?}"
    );
    // Cache reflects new content.
    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, "t2");
    // Other section preserved byte-for-byte.
    let spec_got = ops.read_section(IssueNumber(1), &n("spec")).unwrap();
    assert_eq!(spec_got, "v1");
}

// RED-63: write_section adding a new section (not in original body) creates a comment
#[test]
fn red_63_write_section_new_section_creates_comment() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    // Add a new plan section — routing defaults plan to comment.
    ops.write_section(IssueNumber(1), &n("plan"), "plan body")
        .unwrap();

    let log = ops.client().call_log();
    let create_comment_count = log
        .iter()
        .filter(|l| l.starts_with("create_comment:#1"))
        .count();
    let patch_body_count = log
        .iter()
        .filter(|l| l.starts_with("patch_body:#1"))
        .count();
    assert_eq!(create_comment_count, 1);
    // Body must be patched at least once (to update index map with the new comment id).
    assert!(patch_body_count >= 1);

    let got = ops.read_section(IssueNumber(1), &n("plan")).unwrap();
    assert_eq!(got, "plan body");
}

// RED-64: write_section failure leaves cache unchanged
#[test]
fn red_64_write_section_failure_preserves_cache() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    // Force a BodyTooLarge failure by writing content that, after assembly,
    // explodes the body size beyond 65,536 bytes.
    let huge = "x".repeat(70_000);
    let res = ops.write_section(IssueNumber(1), &n("tasks"), &huge);
    assert!(res.is_err(), "expected write to fail, got Ok");

    // Cache should still show the original content.
    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, "t1");
}

// RED-65: create_spec call order: create_issue -> create_comment* -> patch_body
#[test]
fn red_65_create_spec_call_order() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();

    let ops = SpecOps::new(client, cache);
    let mut sections = std::collections::BTreeMap::new();
    sections.insert(n("spec"), "spec content".to_string());
    sections.insert(n("tasks"), "- [ ] T-001".to_string());
    sections.insert(n("plan"), "plan body".to_string());

    let snap = ops
        .create_spec("Test SPEC", sections, &["phase/draft".to_string()])
        .unwrap();

    let log = ops.client().call_log();
    let create_issue_idx = log
        .iter()
        .position(|l| l.starts_with("create_issue:"))
        .unwrap();
    let create_comment_idx = log
        .iter()
        .position(|l| l.starts_with("create_comment:"))
        .unwrap();
    let patch_body_idx = log
        .iter()
        .position(|l| l.starts_with("patch_body:"))
        .unwrap();

    assert!(create_issue_idx < create_comment_idx);
    assert!(create_comment_idx < patch_body_idx);

    // Labels include both gwt-spec and phase/draft.
    assert!(snap.labels.contains(&"gwt-spec".to_string()));
    assert!(snap.labels.contains(&"phase/draft".to_string()));
}

// RED-66: read_section for missing section returns SectionNotFound
#[test]
fn red_66_read_missing_section_errors() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "s", "t");

    let ops = SpecOps::new(client, cache);
    let err = ops.read_section(IssueNumber(1), &n("plan")).unwrap_err();
    assert!(matches!(err, SpecOpsError::SectionNotFound(_)));
}
