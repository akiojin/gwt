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

// RED-64 (reshaped by SPEC-3248 P7C / #3284): oversized section content is no
// longer a hard failure — the supported multipart writer splits it across
// comments. A write that cannot be performed safely still leaves the cache
// unchanged (see red_92_multipart_failure_zero_partial_overwrite).
#[test]
fn red_64_write_section_oversized_succeeds_via_multipart() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    let huge = oversized_content(70_000);
    ops.write_section(IssueNumber(1), &n("tasks"), &huge)
        .unwrap();

    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, huge);
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

// ---------------------------------------------------------------------------
// SPEC-3248 P7C bootstrap (#3284): supported multipart section writer
// ---------------------------------------------------------------------------

/// Multi-line content of roughly `target` bytes so the splitter has safe cut
/// points at line boundaries.
fn oversized_content(target: usize) -> String {
    let line = "- [ ] T-xxx multipart writer fixture line with enough text to be realistic\n";
    line.repeat(target / line.len() + 1)
        .trim_end_matches('\n')
        .to_string()
}

// RED-90: oversized write splits into part-marked comments, records every
// comment id in the sections index in part order, and round-trips exactly.
#[test]
fn red_90_multipart_write_creates_part_comments() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    let huge = oversized_content(70_000);
    ops.write_section(IssueNumber(1), &n("tasks"), &huge)
        .unwrap();

    let comments = ops.client().comments(IssueNumber(1));
    let part_comments: Vec<_> = comments
        .iter()
        .filter(|c| c.body.contains("<!-- artifact:tasks BEGIN part="))
        .collect();
    assert!(
        part_comments.len() >= 2,
        "expected >= 2 part comments, got {}",
        part_comments.len()
    );
    for c in &comments {
        assert!(
            c.body.len() <= 65_536,
            "comment body exceeds GitHub limit: {} bytes",
            c.body.len()
        );
    }
    assert!(
        part_comments[0].body.contains("part=1/"),
        "first part comment must carry part=1 marker"
    );

    // Round-trip through a fresh read.
    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, huge);
}

// SPEC-3248 P7C T-274: the write receipt carries the operability facts the
// Artifact Operability Record persists — resident location, comment ids in
// part order, and the largest single part payload.
#[test]
fn t274_write_receipt_carries_operability_fields() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    // Body-resident write: no comment ids, one implicit part.
    let receipt = ops
        .write_section(IssueNumber(1), &n("tasks"), "t2")
        .unwrap();
    assert_eq!(receipt.location, "body");
    assert!(receipt.comment_ids.is_empty());
    assert_eq!(receipt.largest_part_bytes, receipt.bytes);

    // Multipart comment-resident write: ids in part order, bounded parts.
    let huge = oversized_content(70_000);
    let receipt = ops
        .write_section(IssueNumber(1), &n("tasks"), &huge)
        .unwrap();
    assert!(
        receipt.parts >= 2,
        "expected multipart, got {}",
        receipt.parts
    );
    assert_eq!(receipt.location, "comments");
    assert_eq!(receipt.comment_ids.len(), receipt.parts);
    assert!(receipt.largest_part_bytes <= 65_536);
    assert!(receipt.largest_part_bytes <= receipt.bytes);
    // The recorded ids must be exactly the part comments, in order.
    let comments = ops.client().comments(IssueNumber(1));
    for (i, id) in receipt.comment_ids.iter().enumerate() {
        let comment = comments.iter().find(|c| c.id.0 == *id).unwrap();
        assert!(
            comment.body.contains(&format!("part={}/", i + 1)),
            "comment {id} must carry part={} marker",
            i + 1
        );
    }
}

// RED-91: shrinking a multipart section back to a small single-comment
// section swaps to one fresh comment and deletes the stale part comments.
#[test]
fn red_91_multipart_shrink_deletes_stale_parts() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    let huge = oversized_content(70_000);
    ops.write_section(IssueNumber(1), &n("tasks"), &huge)
        .unwrap();
    let before = ops.client().comments(IssueNumber(1));
    assert!(before.len() >= 2);

    ops.write_section(IssueNumber(1), &n("tasks"), &oversized_content(20_000))
        .unwrap();

    let log = ops.client().call_log();
    assert!(
        log.iter().any(|l| l.starts_with("delete_comment:")),
        "stale part comments must be deleted: {log:?}"
    );
    let after = ops.client().comments(IssueNumber(1));
    let tasks_comments: Vec<_> = after
        .iter()
        .filter(|c| c.body.contains("<!-- artifact:tasks BEGIN"))
        .collect();
    assert_eq!(
        tasks_comments.len(),
        1,
        "expected exactly one tasks comment after shrink"
    );
    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, oversized_content(20_000));
}

// RED-92: a failure while creating part comments must not touch the section
// index — readers keep seeing the previous content (zero partial overwrite).
#[test]
fn red_92_multipart_failure_zero_partial_overwrite() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    // Establish tasks as a comment-resident section first.
    ops.write_section(IssueNumber(1), &n("tasks"), &oversized_content(20_000))
        .unwrap();

    // Now inject a failure on the second part creation of the next write.
    ops.client().fail_create_comment_after(1);
    let huge = oversized_content(70_000);
    let err = ops.write_section(IssueNumber(1), &n("tasks"), &huge);
    assert!(err.is_err(), "expected injected create failure to surface");

    // The section must still read back as the previous content.
    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, oversized_content(20_000));
}

// RED-93: post-write readback verifies the remote content actually matches
// what was written; server-side corruption fails closed and rolls back.
#[test]
fn red_93_readback_mismatch_fails_closed() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    ops.write_section(IssueNumber(1), &n("tasks"), &oversized_content(20_000))
        .unwrap();

    ops.client().corrupt_next_create_comment();
    let huge = oversized_content(70_000);
    let err = ops
        .write_section(IssueNumber(1), &n("tasks"), &huge)
        .unwrap_err();
    assert!(
        matches!(err, SpecOpsError::ReadbackMismatch { .. }),
        "expected ReadbackMismatch, got other error"
    );

    // Rollback: the previous content must still be readable.
    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, oversized_content(20_000));
}

// RED-94: small single-comment sections keep the in-place patch behavior
// (stable comment id, no create/delete churn).
#[test]
fn red_94_small_section_keeps_in_place_patch() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let ops = SpecOps::new(client, cache);
    ops.write_section(IssueNumber(1), &n("plan"), "plan v1")
        .unwrap();
    let first_id = ops.client().comments(IssueNumber(1))[0].id;

    ops.write_section(IssueNumber(1), &n("plan"), "plan v2")
        .unwrap();

    let log = ops.client().call_log();
    assert!(
        log.iter()
            .any(|l| l.starts_with(&format!("patch_comment:comment:{}", first_id.0))),
        "expected in-place patch of the existing comment: {log:?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("delete_comment:")),
        "no delete expected for single->single rewrite: {log:?}"
    );
    let comments = ops.client().comments(IssueNumber(1));
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].id, first_id, "comment id must stay stable");
    let got = ops.read_section(IssueNumber(1), &n("plan")).unwrap();
    assert_eq!(got, "plan v2");
}

// RED-95: create_spec with an oversized section materializes it as multipart
// comments and round-trips exactly.
#[test]
fn red_95_create_spec_with_oversized_section() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();

    let ops = SpecOps::new(client, cache);
    let huge = oversized_content(70_000);
    let mut sections = std::collections::BTreeMap::new();
    sections.insert(n("spec"), "spec content".to_string());
    sections.insert(n("plan"), huge.clone());

    let snap = ops.create_spec("Oversized SPEC", sections, &[]).unwrap();

    let comments = ops.client().comments(snap.number);
    for c in &comments {
        assert!(c.body.len() <= 65_536);
    }
    let got = ops.read_section(snap.number, &n("plan")).unwrap();
    assert_eq!(got, huge);
}

// RED-96: #3248-scale roundtrip — ~130 KiB of mixed-width markdown survives
// write -> remote storage -> readback byte-for-byte across >= 3 parts.
#[test]
fn red_96_spec_3248_scale_roundtrip() {
    let tmp = TempDir::new().unwrap();
    let cache = Cache::new(tmp.path().to_path_buf());
    let client = FakeIssueClient::new();
    seed_spec(&client, &cache, 1, "v1", "t1");

    let block = "## 2026-07-07 Tasks: 統合ゲート強化\n\n- [ ] **T-100** 検証コマンドと結果を記録する。日本語の説明文を含む現実的なタスク行。\n- [ ] **T-101** stale evidence を拒否し、fail-closed で報告する。\n\n";
    let mut content = String::new();
    while content.len() < 130 * 1024 {
        content.push_str(block);
    }
    let content = content.trim_end_matches('\n').to_string();

    let ops = SpecOps::new(client, cache);
    ops.write_section(IssueNumber(1), &n("tasks"), &content)
        .unwrap();

    let comments = ops.client().comments(IssueNumber(1));
    let parts: Vec<_> = comments
        .iter()
        .filter(|c| c.body.contains("<!-- artifact:tasks BEGIN part="))
        .collect();
    assert!(parts.len() >= 3, "expected >= 3 parts, got {}", parts.len());
    for c in &comments {
        assert!(c.body.len() <= 65_536);
    }
    let got = ops.read_section(IssueNumber(1), &n("tasks")).unwrap();
    assert_eq!(got, content);
}
