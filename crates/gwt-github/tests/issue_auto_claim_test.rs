use gwt_github::issue_auto_claim::{
    acquire_claim, claim_is_active, parse_claim_comment, render_claim_comment,
    select_winning_claim, ClaimAcquireOutcome, ClaimComment, ClaimStatus,
};
use gwt_github::{
    CommentId, CommentSnapshot, FakeIssueClient, IssueNumber, IssueSnapshot, IssueState, UpdatedAt,
};

fn claim(id: &str, owner: &str, heartbeat: &str, expires: &str) -> ClaimComment {
    ClaimComment {
        comment_id: Some(CommentId(100)),
        claim_id: id.to_string(),
        owner: owner.to_string(),
        issue_number: 42,
        status: ClaimStatus::Active,
        heartbeat_at: heartbeat.to_string(),
        expires_at: expires.to_string(),
        launched_work_id: Some("work/issue-42".to_string()),
    }
}

fn snapshot(comments: Vec<CommentSnapshot>) -> IssueSnapshot {
    IssueSnapshot {
        number: IssueNumber(42),
        title: "Improve automatically".to_string(),
        body: String::new(),
        labels: vec!["auto-improve".to_string()],
        state: IssueState::Open,
        updated_at: UpdatedAt::new("t1"),
        comments,
    }
}

fn comment(id: u64, claim: &ClaimComment) -> CommentSnapshot {
    CommentSnapshot {
        id: CommentId(id),
        body: render_claim_comment(claim),
        updated_at: UpdatedAt::new("t1"),
    }
}

#[test]
fn claim_comment_round_trips_machine_readable_payload() {
    let original = claim(
        "claim-a",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );

    let rendered = render_claim_comment(&original);
    let parsed = parse_claim_comment(Some(CommentId(777)), &rendered).expect("claim parses");

    assert_eq!(parsed.comment_id, Some(CommentId(777)));
    assert_eq!(parsed.claim_id, original.claim_id);
    assert_eq!(parsed.owner, original.owner);
    assert_eq!(parsed.issue_number, 42);
    assert_eq!(parsed.status, ClaimStatus::Active);
    assert_eq!(parsed.heartbeat_at, original.heartbeat_at);
    assert_eq!(parsed.expires_at, original.expires_at);
    assert_eq!(parsed.launched_work_id, Some("work/issue-42".to_string()));
}

#[test]
fn active_claim_requires_active_status_and_future_expiry() {
    let active = claim(
        "claim-a",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    let mut released = active.clone();
    released.status = ClaimStatus::Released;

    assert!(claim_is_active(&active, "2026-06-23T10:29:59Z"));
    assert!(!claim_is_active(&active, "2026-06-23T10:30:00Z"));
    assert!(!claim_is_active(&released, "2026-06-23T10:29:59Z"));
}

#[test]
fn winner_selection_ignores_stale_claims_and_picks_oldest_active() {
    let stale = claim(
        "claim-stale",
        "host-a/session-a",
        "2026-06-23T09:00:00Z",
        "2026-06-23T09:30:00Z",
    );
    let newer = claim(
        "claim-newer",
        "host-c/session-c",
        "2026-06-23T10:02:00Z",
        "2026-06-23T10:32:00Z",
    );
    let older = claim(
        "claim-older",
        "host-b/session-b",
        "2026-06-23T10:01:00Z",
        "2026-06-23T10:31:00Z",
    );

    let claims = [stale, newer, older];
    let winner = select_winning_claim(&claims, "2026-06-23T10:05:00Z").expect("winner");

    assert_eq!(winner.claim_id, "claim-older");
}

#[test]
fn acquire_claim_creates_comment_and_confirms_winner() {
    let client = FakeIssueClient::new();
    client.seed(snapshot(vec![]));
    let requested = claim(
        "claim-a",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );

    let outcome = acquire_claim(&client, IssueNumber(42), requested, "2026-06-23T10:01:00Z")
        .expect("claim acquired");

    match outcome {
        ClaimAcquireOutcome::Acquired(acquired) => {
            assert_eq!(acquired.comment_id, Some(CommentId(1)));
            assert_eq!(acquired.claim_id, "claim-a");
        }
        other => panic!("expected acquired, got {other:?}"),
    }
    assert_eq!(
        client.call_log(),
        vec!["fetch:#42", "create_comment:#42", "fetch:#42"]
    );
}

#[test]
fn acquire_claim_does_not_create_when_other_active_claim_wins() {
    let client = FakeIssueClient::new();
    let other = claim(
        "claim-other",
        "host-b/session-b",
        "2026-06-23T09:59:00Z",
        "2026-06-23T10:30:00Z",
    );
    client.seed(snapshot(vec![comment(9, &other)]));
    let requested = claim(
        "claim-a",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );

    let outcome = acquire_claim(&client, IssueNumber(42), requested, "2026-06-23T10:01:00Z")
        .expect("claim checked");

    match outcome {
        ClaimAcquireOutcome::Blocked(blocking) => {
            assert_eq!(blocking.comment_id, Some(CommentId(9)));
            assert_eq!(blocking.claim_id, "claim-other");
        }
        other => panic!("expected blocked, got {other:?}"),
    }
    assert_eq!(client.call_log(), vec!["fetch:#42"]);
}

#[test]
fn acquire_claim_refreshes_existing_claim_for_same_owner() {
    let client = FakeIssueClient::new();
    let existing = claim(
        "claim-old",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    client.seed(snapshot(vec![comment(9, &existing)]));
    let requested = claim(
        "claim-new",
        "host-a/session-a",
        "2026-06-23T10:05:00Z",
        "2026-06-23T10:35:00Z",
    );

    let outcome = acquire_claim(&client, IssueNumber(42), requested, "2026-06-23T10:06:00Z")
        .expect("claim refreshed");

    match outcome {
        ClaimAcquireOutcome::Acquired(acquired) => {
            assert_eq!(acquired.comment_id, Some(CommentId(9)));
            assert_eq!(acquired.claim_id, "claim-new");
            assert_eq!(acquired.expires_at, "2026-06-23T10:35:00Z");
        }
        other => panic!("expected refreshed acquisition, got {other:?}"),
    }
    assert_eq!(
        client.call_log(),
        vec!["fetch:#42", "patch_comment:comment:9"]
    );
}
