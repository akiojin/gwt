//! Contract tests for the `IssueClient` trait via [`FakeIssueClient`]
//! (SPEC-12 tdd.md Layer 4).

use gwt_github::client::fake::FakeIssueClient;
use gwt_github::client::{
    ApiError, CommentId, CommentSnapshot, FetchResult, IssueClient, IssueNumber, IssueSnapshot,
    IssueState, SpecListFilter, UpdatedAt,
};

fn seed_simple(client: &FakeIssueClient, number: u64, title: &str, body: &str) -> IssueSnapshot {
    let snapshot = IssueSnapshot {
        number: IssueNumber(number),
        title: title.to_string(),
        body: body.to_string(),
        labels: vec!["gwt-spec".to_string(), "phase/review".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("seeded"),
        comments: Vec::new(),
    };
    client.seed(snapshot.clone());
    snapshot
}

// RED-30: first fetch returns Updated
#[test]
fn red_30_first_fetch_returns_updated() {
    let c = FakeIssueClient::new();
    seed_simple(&c, 2001, "TUI theme", "body v1");
    let res = c.fetch(IssueNumber(2001), None).unwrap();
    match res {
        FetchResult::Updated(snap) => {
            assert_eq!(snap.number, IssueNumber(2001));
            assert_eq!(snap.body, "body v1");
        }
        _ => panic!("expected Updated"),
    }
}

// RED-31: fetch with matching updated_at returns NotModified
#[test]
fn red_31_conditional_fetch_not_modified() {
    let c = FakeIssueClient::new();
    let seeded = seed_simple(&c, 2001, "x", "b");
    let res = c
        .fetch(IssueNumber(2001), Some(&seeded.updated_at))
        .unwrap();
    assert!(matches!(res, FetchResult::NotModified));
}

// RED-32: fetch after mutation returns Updated with fresh updated_at
#[test]
fn red_32_fetch_after_mutation_returns_updated() {
    let c = FakeIssueClient::new();
    let seeded = seed_simple(&c, 2001, "x", "b");
    c.patch_body(IssueNumber(2001), "b v2").unwrap();
    let res = c
        .fetch(IssueNumber(2001), Some(&seeded.updated_at))
        .unwrap();
    match res {
        FetchResult::Updated(snap) => {
            assert_eq!(snap.body, "b v2");
            assert_ne!(snap.updated_at, seeded.updated_at);
        }
        _ => panic!("expected Updated after patch"),
    }
}

// RED-33: fetch unknown issue returns NotFound
#[test]
fn red_33_fetch_unknown_returns_not_found() {
    let c = FakeIssueClient::new();
    let err = c.fetch(IssueNumber(9999), None).unwrap_err();
    assert!(matches!(err, ApiError::NotFound(IssueNumber(9999))));
}

// RED-34: patch_body too large -> BodyTooLarge
#[test]
fn red_34_patch_body_too_large() {
    let c = FakeIssueClient::new();
    seed_simple(&c, 1, "x", "b");
    let big = "x".repeat(70_000);
    let err = c.patch_body(IssueNumber(1), &big).unwrap_err();
    assert!(matches!(err, ApiError::BodyTooLarge));
}

// RED-35: create_issue returns new number starting at 1 on empty fake
#[test]
fn red_35_create_issue_assigns_number() {
    let c = FakeIssueClient::new();
    let labels = vec!["gwt-spec".to_string(), "phase/draft".to_string()];
    let a = c.create_issue("A", "body a", &labels).unwrap();
    let b = c.create_issue("B", "body b", &labels).unwrap();
    assert_eq!(a.number, IssueNumber(1));
    assert_eq!(b.number, IssueNumber(2));
    assert_eq!(a.state, IssueState::Open);
    assert_eq!(a.labels, labels);
}

// RED-36: create_comment increments id and attaches to the target issue
#[test]
fn red_36_create_comment_assigns_id() {
    let c = FakeIssueClient::new();
    seed_simple(&c, 10, "x", "b");
    let a = c.create_comment(IssueNumber(10), "first").unwrap();
    let b = c.create_comment(IssueNumber(10), "second").unwrap();
    assert_ne!(a.id, b.id);
    assert_eq!(a.body, "first");
    assert_eq!(b.body, "second");
    // Confirm both are attached to the issue.
    let fetched = c.fetch(IssueNumber(10), None).unwrap();
    match fetched {
        FetchResult::Updated(snap) => {
            assert_eq!(snap.comments.len(), 2);
            assert_eq!(snap.comments[0].body, "first");
            assert_eq!(snap.comments[1].body, "second");
        }
        _ => panic!("expected Updated"),
    }
}

// RED-37: patch_comment targets the correct comment by id
#[test]
fn red_37_patch_comment_by_id() {
    let c = FakeIssueClient::new();
    seed_simple(&c, 42, "x", "b");
    let first = c.create_comment(IssueNumber(42), "v1").unwrap();
    let patched: CommentSnapshot = c.patch_comment(first.id, "v2").unwrap();
    assert_eq!(patched.id, first.id);
    assert_eq!(patched.body, "v2");
    assert_ne!(patched.updated_at, first.updated_at);
}

// RED-38: patch_comment for unknown id returns CommentNotFound
#[test]
fn red_38_patch_unknown_comment() {
    let c = FakeIssueClient::new();
    let err = c.patch_comment(CommentId(9999), "x").unwrap_err();
    assert!(matches!(err, ApiError::CommentNotFound(CommentId(9999))));
}

// RED-39: set_labels replaces label set
#[test]
fn red_39_set_labels_replaces() {
    let c = FakeIssueClient::new();
    let seeded = seed_simple(&c, 5, "x", "b");
    assert_eq!(seeded.labels, vec!["gwt-spec", "phase/review"]);
    let new_labels = vec!["gwt-spec".to_string(), "phase/done".to_string()];
    let updated = c.set_labels(IssueNumber(5), &new_labels).unwrap();
    assert_eq!(updated.labels, new_labels);
}

// RED-40: set_state transitions open -> closed
#[test]
fn red_40_set_state_transitions() {
    let c = FakeIssueClient::new();
    seed_simple(&c, 7, "x", "b");
    let closed = c.set_state(IssueNumber(7), IssueState::Closed).unwrap();
    assert_eq!(closed.state, IssueState::Closed);
    let reopened = c.set_state(IssueNumber(7), IssueState::Open).unwrap();
    assert_eq!(reopened.state, IssueState::Open);
}

// RED-41: list_spec_issues filters by phase
#[test]
fn red_41_list_spec_issues_phase_filter() {
    let c = FakeIssueClient::new();
    seed_spec(&c, 1, "a", "phase/draft");
    seed_spec(&c, 2, "b", "phase/implementation");
    seed_spec(&c, 3, "c", "phase/implementation");
    seed_spec(&c, 4, "d", "phase/done");
    let filter = SpecListFilter {
        phase: Some("implementation".to_string()),
        state: None,
    };
    let list = c.list_spec_issues(&filter).unwrap();
    let numbers: Vec<u64> = list.iter().map(|s| s.number.0).collect();
    assert_eq!(numbers, vec![3, 2]); // desc order by number
}

// RED-42: list_spec_issues filters by state
#[test]
fn red_42_list_spec_issues_state_filter() {
    let c = FakeIssueClient::new();
    seed_spec(&c, 1, "a", "phase/done");
    seed_spec(&c, 2, "b", "phase/implementation");
    c.set_state(IssueNumber(1), IssueState::Closed).unwrap();
    let filter = SpecListFilter {
        phase: None,
        state: Some(IssueState::Closed),
    };
    let list = c.list_spec_issues(&filter).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].number, IssueNumber(1));
}

// RED-43: list_spec_issues excludes Issues without `gwt-spec` label
#[test]
fn red_43_list_requires_gwt_spec_label() {
    let c = FakeIssueClient::new();
    seed_spec(&c, 1, "a", "phase/draft");
    // Seed a non-spec Issue.
    c.seed(IssueSnapshot {
        number: IssueNumber(2),
        title: "plain".to_string(),
        body: String::new(),
        labels: vec!["bug".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t0"),
        comments: Vec::new(),
    });
    let list = c.list_spec_issues(&SpecListFilter::default()).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].number, IssueNumber(1));
}

// RED-44: call log records operations in order
#[test]
fn red_44_call_log_records_operations() {
    let c = FakeIssueClient::new();
    seed_simple(&c, 1, "x", "b");
    let _ = c.fetch(IssueNumber(1), None).unwrap();
    let _ = c.patch_body(IssueNumber(1), "new").unwrap();
    let _ = c.list_spec_issues(&SpecListFilter::default()).unwrap();
    let log = c.call_log();
    assert_eq!(
        log,
        vec!["fetch:#1", "patch_body:#1", "list_spec_issues:*",]
    );
}

// RED-45: create_issue returns snapshot with empty comment list
#[test]
fn red_45_create_issue_has_no_comments() {
    let c = FakeIssueClient::new();
    let issue = c.create_issue("T", "B", &["gwt-spec".to_string()]).unwrap();
    assert!(issue.comments.is_empty());
}

// Helper: seed a SPEC-labeled issue with an extra phase label.
fn seed_spec(client: &FakeIssueClient, number: u64, title: &str, phase_label: &str) {
    client.seed(IssueSnapshot {
        number: IssueNumber(number),
        title: title.to_string(),
        body: String::new(),
        labels: vec!["gwt-spec".to_string(), phase_label.to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new(format!("seed-{number}")),
        comments: Vec::new(),
    });
}
