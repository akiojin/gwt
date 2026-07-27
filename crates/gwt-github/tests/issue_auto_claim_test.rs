use gwt_github::issue_auto_claim::{
    acquire_claim, acquire_claim_mutation, claim_is_active, parse_claim_comment, release_claim,
    render_claim_comment, select_winning_claim, ClaimAcquireOutcome, ClaimComment,
    ClaimReleaseOutcome, ClaimStatus,
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

#[test]
fn stable_claim_id_replay_reuses_existing_winner_without_duplicate_logical_launch() {
    let client = FakeIssueClient::new();
    let existing = claim(
        "stable-effect-claim",
        "host-a/old-process",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    client.seed(snapshot(vec![comment(9, &existing)]));
    let replayed = claim(
        "stable-effect-claim",
        "host-a/restarted-process",
        "2026-06-23T10:05:00Z",
        "2026-06-23T10:35:00Z",
    );

    for _ in 0..2 {
        let outcome = acquire_claim(
            &client,
            IssueNumber(42),
            replayed.clone(),
            "2026-06-23T10:06:00Z",
        )
        .expect("stable claim replay reconciles");
        match outcome {
            ClaimAcquireOutcome::Acquired(acquired) => {
                assert_eq!(acquired.comment_id, Some(CommentId(9)));
                assert_eq!(acquired.claim_id, "stable-effect-claim");
            }
            other => panic!("expected existing logical acquisition, got {other:?}"),
        }
    }

    assert_eq!(
        client.call_log(),
        vec!["fetch:#42", "fetch:#42"],
        "restart replay must neither create nor patch a duplicate claim"
    );
    assert_eq!(client.comments(IssueNumber(42)).len(), 1);
}

#[test]
fn expired_stable_claim_never_overrides_a_current_foreign_winner() {
    let client = FakeIssueClient::new();
    let expired_own = claim(
        "stable-effect-claim",
        "host-a/old-process",
        "2026-06-23T09:00:00Z",
        "2026-06-23T09:30:00Z",
    );
    let foreign = claim(
        "foreign-current-claim",
        "host-b/process",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    client.seed(snapshot(vec![
        comment(9, &expired_own),
        comment(10, &foreign),
    ]));
    let replayed = claim(
        "stable-effect-claim",
        "host-a/restarted-process",
        "2026-06-23T10:05:00Z",
        "2026-06-23T10:35:00Z",
    );

    let outcome =
        acquire_claim_mutation(&client, IssueNumber(42), replayed, "2026-06-23T10:05:00Z")
            .expect("foreign winner is an ordinary blocked outcome");

    assert!(matches!(
        outcome,
        ClaimAcquireOutcome::Blocked(ref winner) if winner.claim_id == "foreign-current-claim"
    ));
    assert_eq!(
        client.call_log(),
        vec!["fetch:#42"],
        "the expired stable claim is not reactivated while another lease is live"
    );
}

#[test]
fn expired_stable_claim_is_renewed_and_read_back_without_duplicate_comment() {
    let client = FakeIssueClient::new();
    let expired = claim(
        "stable-effect-claim",
        "host-a/old-process",
        "2026-06-23T09:00:00Z",
        "2026-06-23T09:30:00Z",
    );
    client.seed(snapshot(vec![comment(9, &expired)]));
    let replayed = claim(
        "stable-effect-claim",
        "host-a/restarted-process",
        "2026-06-23T10:05:00Z",
        "2026-06-23T10:35:00Z",
    );

    let outcome =
        acquire_claim_mutation(&client, IssueNumber(42), replayed, "2026-06-23T10:05:00Z")
            .expect("stable claim renews");

    assert!(matches!(outcome, ClaimAcquireOutcome::Acquired(_)));
    assert_eq!(client.comments(IssueNumber(42)).len(), 1);
    assert_eq!(
        client.call_log(),
        vec!["fetch:#42", "patch_comment:comment:9", "fetch:#42"]
    );
}

#[test]
fn releasing_same_stable_claim_id_twice_is_idempotent() {
    let client = FakeIssueClient::new();
    let existing = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    client.seed(snapshot(vec![comment(9, &existing)]));

    let first = release_claim(&client, IssueNumber(42), "stable-effect-claim")
        .expect("first release succeeds");
    assert!(matches!(first, ClaimReleaseOutcome::Released(_)));

    let second = release_claim(&client, IssueNumber(42), "stable-effect-claim")
        .expect("replayed release succeeds");
    assert!(matches!(second, ClaimReleaseOutcome::AlreadyReleased(_)));

    let stored = client.comments(IssueNumber(42));
    assert_eq!(stored.len(), 1);
    let released = parse_claim_comment(Some(stored[0].id), &stored[0].body)
        .expect("released claim remains machine-readable");
    assert_eq!(released.claim_id, "stable-effect-claim");
    assert_eq!(released.status, ClaimStatus::Released);
    assert_eq!(
        client.call_log(),
        vec!["fetch:#42", "patch_comment:comment:9", "fetch:#42"],
        "second release observes the target state and does not patch again"
    );
}
