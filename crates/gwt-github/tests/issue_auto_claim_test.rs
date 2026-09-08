use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use gwt_github::client::{OwnerMutationError, OwnerMutationResult};
use gwt_github::issue_auto_claim::{
    acquire_claim, acquire_claim_mutation, claim_is_active, parse_claim_comment, release_claim,
    release_claim_mutation, render_claim_comment, select_winning_claim, ClaimAcquireOutcome,
    ClaimComment, ClaimReleaseOutcome, ClaimStatus,
};
use gwt_github::{
    ApiError, CommentId, CommentSnapshot, FakeIssueClient, FetchResult, IssueClient, IssueNumber,
    IssueSnapshot, IssueState, SpecListFilter, SpecSummary, UpdatedAt,
};

struct PatchFaultClient {
    inner: FakeIssueClient,
    fail_at: Option<usize>,
    inject_after_first_patch: Option<(IssueNumber, String)>,
    patch_attempts: AtomicUsize,
    injected: AtomicBool,
}

impl PatchFaultClient {
    fn fail_at(inner: FakeIssueClient, attempt: usize) -> Self {
        Self {
            inner,
            fail_at: Some(attempt),
            inject_after_first_patch: None,
            patch_attempts: AtomicUsize::new(0),
            injected: AtomicBool::new(false),
        }
    }

    fn inject_after_first_patch(
        inner: FakeIssueClient,
        issue_number: IssueNumber,
        claim: &ClaimComment,
    ) -> Self {
        Self {
            inner,
            fail_at: None,
            inject_after_first_patch: Some((issue_number, render_claim_comment(claim))),
            patch_attempts: AtomicUsize::new(0),
            injected: AtomicBool::new(false),
        }
    }

    fn patch_once(
        &self,
        comment_id: CommentId,
        new_body: &str,
    ) -> Result<CommentSnapshot, ApiError> {
        let attempt = self.patch_attempts.fetch_add(1, Ordering::SeqCst) + 1;
        if self.fail_at == Some(attempt) {
            return Err(ApiError::Network(format!(
                "injected pre-submit patch failure at attempt {attempt}"
            )));
        }
        let patched = self.inner.patch_comment(comment_id, new_body)?;
        if let Some((issue_number, body)) = &self.inject_after_first_patch {
            if !self.injected.swap(true, Ordering::SeqCst) {
                self.inner.create_comment(*issue_number, body)?;
            }
        }
        Ok(patched)
    }
}

impl IssueClient for PatchFaultClient {
    fn fetch(
        &self,
        number: IssueNumber,
        since: Option<&UpdatedAt>,
    ) -> Result<FetchResult, ApiError> {
        self.inner.fetch(number, since)
    }

    fn patch_body(&self, _number: IssueNumber, _new_body: &str) -> Result<IssueSnapshot, ApiError> {
        unreachable!("unused by claim fault tests")
    }

    fn patch_title(
        &self,
        _number: IssueNumber,
        _new_title: &str,
    ) -> Result<IssueSnapshot, ApiError> {
        unreachable!("unused by claim fault tests")
    }

    fn patch_comment(
        &self,
        comment_id: CommentId,
        new_body: &str,
    ) -> Result<CommentSnapshot, ApiError> {
        self.patch_once(comment_id, new_body)
    }

    fn patch_comment_mutation(
        &self,
        comment_id: CommentId,
        new_body: &str,
    ) -> OwnerMutationResult<CommentSnapshot> {
        self.patch_once(comment_id, new_body)
            .map_err(OwnerMutationError::PreSubmit)
    }

    fn create_comment(
        &self,
        _number: IssueNumber,
        _body: &str,
    ) -> Result<CommentSnapshot, ApiError> {
        unreachable!("unused by claim fault tests")
    }

    fn delete_comment(&self, _comment_id: CommentId) -> Result<(), ApiError> {
        unreachable!("unused by claim fault tests")
    }

    fn create_issue(
        &self,
        _title: &str,
        _body: &str,
        _labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        unreachable!("unused by claim fault tests")
    }

    fn set_labels(
        &self,
        _number: IssueNumber,
        _labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        unreachable!("unused by claim fault tests")
    }

    fn set_state(
        &self,
        _number: IssueNumber,
        _state: IssueState,
    ) -> Result<IssueSnapshot, ApiError> {
        unreachable!("unused by claim fault tests")
    }

    fn list_spec_issues(&self, _filter: &SpecListFilter) -> Result<Vec<SpecSummary>, ApiError> {
        unreachable!("unused by claim fault tests")
    }
}

struct OmittedClaimReadbackClient {
    inner: FakeIssueClient,
    post_submission_snapshot: IssueSnapshot,
    fetches: AtomicUsize,
}

impl OmittedClaimReadbackClient {
    fn new(initial_snapshot: IssueSnapshot, post_submission_snapshot: IssueSnapshot) -> Self {
        let inner = FakeIssueClient::new();
        inner.seed(initial_snapshot);
        Self {
            inner,
            post_submission_snapshot,
            fetches: AtomicUsize::new(0),
        }
    }
}

impl IssueClient for OmittedClaimReadbackClient {
    fn fetch(
        &self,
        number: IssueNumber,
        since: Option<&UpdatedAt>,
    ) -> Result<FetchResult, ApiError> {
        if self.fetches.fetch_add(1, Ordering::SeqCst) == 0 {
            self.inner.fetch(number, since)
        } else {
            Ok(FetchResult::Updated(self.post_submission_snapshot.clone()))
        }
    }

    fn patch_body(&self, number: IssueNumber, new_body: &str) -> Result<IssueSnapshot, ApiError> {
        self.inner.patch_body(number, new_body)
    }

    fn patch_title(&self, number: IssueNumber, new_title: &str) -> Result<IssueSnapshot, ApiError> {
        self.inner.patch_title(number, new_title)
    }

    fn patch_comment(
        &self,
        comment_id: CommentId,
        new_body: &str,
    ) -> Result<CommentSnapshot, ApiError> {
        self.inner.patch_comment(comment_id, new_body)
    }

    fn create_comment(&self, number: IssueNumber, body: &str) -> Result<CommentSnapshot, ApiError> {
        self.inner.create_comment(number, body)
    }

    fn delete_comment(&self, comment_id: CommentId) -> Result<(), ApiError> {
        self.inner.delete_comment(comment_id)
    }

    fn create_issue(
        &self,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        self.inner.create_issue(title, body, labels)
    }

    fn set_labels(
        &self,
        number: IssueNumber,
        labels: &[String],
    ) -> Result<IssueSnapshot, ApiError> {
        self.inner.set_labels(number, labels)
    }

    fn set_state(&self, number: IssueNumber, state: IssueState) -> Result<IssueSnapshot, ApiError> {
        self.inner.set_state(number, state)
    }

    fn list_spec_issues(&self, filter: &SpecListFilter) -> Result<Vec<SpecSummary>, ApiError> {
        self.inner.list_spec_issues(filter)
    }
}

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

fn stored_claim(client: &FakeIssueClient, id: u64) -> ClaimComment {
    let comment = client
        .comments(IssueNumber(42))
        .into_iter()
        .find(|comment| comment.id == CommentId(id))
        .expect("stored comment exists");
    parse_claim_comment(Some(comment.id), &comment.body).expect("stored claim parses")
}

#[derive(Clone, Copy)]
enum AcquirePath {
    Legacy,
    Mutation,
}

#[derive(Clone, Copy)]
enum SubmissionKind {
    Create,
    Refresh,
}

#[derive(Clone, Copy)]
enum OmittedReadbackKind {
    ForeignWinner,
    SameIdCollision,
}

fn acquire_via(
    path: AcquirePath,
    client: &FakeIssueClient,
    requested: ClaimComment,
    now: &str,
) -> ClaimAcquireOutcome {
    match path {
        AcquirePath::Legacy => acquire_claim(client, IssueNumber(42), requested, now)
            .expect("legacy acquisition resolves"),
        AcquirePath::Mutation => acquire_claim_mutation(client, IssueNumber(42), requested, now)
            .expect("mutation acquisition resolves"),
    }
}

fn assert_omitted_submission_readback_fails_closed(
    path: AcquirePath,
    submission: SubmissionKind,
    readback: OmittedReadbackKind,
) {
    let mut requested = claim(
        "submitted-claim",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    requested.comment_id = None;

    let initial_comments = match submission {
        SubmissionKind::Create => Vec::new(),
        SubmissionKind::Refresh => {
            let mut existing = requested.clone();
            existing.status = ClaimStatus::Released;
            vec![comment(9, &existing)]
        }
    };
    let mut other = claim(
        match readback {
            OmittedReadbackKind::ForeignWinner => "foreign-claim",
            OmittedReadbackKind::SameIdCollision => "submitted-claim",
        },
        "host-b/session-b",
        "2026-06-23T09:59:00Z",
        "2026-06-23T10:30:00Z",
    );
    other.comment_id = Some(CommentId(10));
    let client = OmittedClaimReadbackClient::new(
        snapshot(initial_comments),
        snapshot(vec![comment(10, &other)]),
    );

    match path {
        AcquirePath::Legacy => {
            let error = acquire_claim(&client, IssueNumber(42), requested, "2026-06-23T10:01:00Z")
                .expect_err("missing submitted comment must not settle a legacy outcome");
            assert!(matches!(
                error,
                ApiError::Unexpected(message)
                    if message.contains("submitted exact claim")
            ));
        }
        AcquirePath::Mutation => {
            let error =
                acquire_claim_mutation(&client, IssueNumber(42), requested, "2026-06-23T10:01:00Z")
                    .expect_err("missing submitted comment must preserve mutation uncertainty");
            assert!(matches!(
                error,
                OwnerMutationError::RemoteOutcomeUnknown(ApiError::Unexpected(message))
                    if message.contains("submitted exact claim")
            ));
        }
    }

    let expected_call = match submission {
        SubmissionKind::Create => "create_comment:#42",
        SubmissionKind::Refresh => "patch_comment:comment:9",
    };
    assert_eq!(client.inner.call_log(), vec!["fetch:#42", expected_call]);
}

macro_rules! omitted_submission_readback_case {
    ($name:ident, $path:expr, $submission:expr, $readback:expr) => {
        #[test]
        fn $name() {
            assert_omitted_submission_readback_fails_closed($path, $submission, $readback);
        }
    };
}

omitted_submission_readback_case!(
    legacy_create_omits_submitted_comment_with_foreign_winner,
    AcquirePath::Legacy,
    SubmissionKind::Create,
    OmittedReadbackKind::ForeignWinner
);
omitted_submission_readback_case!(
    mutation_create_omits_submitted_comment_with_foreign_winner,
    AcquirePath::Mutation,
    SubmissionKind::Create,
    OmittedReadbackKind::ForeignWinner
);
omitted_submission_readback_case!(
    legacy_refresh_omits_submitted_comment_with_foreign_winner,
    AcquirePath::Legacy,
    SubmissionKind::Refresh,
    OmittedReadbackKind::ForeignWinner
);
omitted_submission_readback_case!(
    mutation_refresh_omits_submitted_comment_with_foreign_winner,
    AcquirePath::Mutation,
    SubmissionKind::Refresh,
    OmittedReadbackKind::ForeignWinner
);
omitted_submission_readback_case!(
    legacy_create_omits_submitted_comment_with_same_id_collision,
    AcquirePath::Legacy,
    SubmissionKind::Create,
    OmittedReadbackKind::SameIdCollision
);
omitted_submission_readback_case!(
    mutation_create_omits_submitted_comment_with_same_id_collision,
    AcquirePath::Mutation,
    SubmissionKind::Create,
    OmittedReadbackKind::SameIdCollision
);
omitted_submission_readback_case!(
    legacy_refresh_omits_submitted_comment_with_same_id_collision,
    AcquirePath::Legacy,
    SubmissionKind::Refresh,
    OmittedReadbackKind::SameIdCollision
);
omitted_submission_readback_case!(
    mutation_refresh_omits_submitted_comment_with_same_id_collision,
    AcquirePath::Mutation,
    SubmissionKind::Refresh,
    OmittedReadbackKind::SameIdCollision
);

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
fn acquire_claim_fails_closed_when_created_comment_cannot_be_read_back() {
    let client = FakeIssueClient::new();
    client.seed(snapshot(vec![]));
    client.corrupt_next_create_comment();
    let requested = claim(
        "claim-a",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );

    let error = acquire_claim(&client, IssueNumber(42), requested, "2026-06-23T10:01:00Z")
        .expect_err("unreadable readback must not acquire the claim");

    assert!(matches!(
        error,
        ApiError::Unexpected(message) if message.contains("submitted exact claim")
    ));
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
fn same_owner_with_a_different_claim_id_cannot_take_over_the_active_winner() {
    for path in [AcquirePath::Legacy, AcquirePath::Mutation] {
        let client = FakeIssueClient::new();
        let mut existing = claim(
            "claim-old",
            "host-a/session-a",
            "2026-06-23T10:00:00Z",
            "2026-06-23T10:30:00Z",
        );
        existing.comment_id = Some(CommentId(9));
        client.seed(snapshot(vec![comment(9, &existing)]));
        let requested = claim(
            "claim-new",
            "host-a/session-a",
            "2026-06-23T10:05:00Z",
            "2026-06-23T10:35:00Z",
        );

        let outcome = acquire_via(path, &client, requested, "2026-06-23T10:06:00Z");

        assert!(matches!(
            outcome,
            ClaimAcquireOutcome::Blocked(ref winner)
                if winner.comment_id == Some(CommentId(9))
                    && winner.claim_id == "claim-old"
                    && winner.owner == "host-a/session-a"
        ));
        assert_eq!(
            client.call_log(),
            vec!["fetch:#42"],
            "a different logical identity must not patch the active winner"
        );
        assert_eq!(stored_claim(&client, 9), existing);
    }
}

#[test]
fn foreign_owner_collision_is_fail_closed_for_every_status_and_expiry() {
    let cases = [
        (ClaimStatus::Active, "2026-06-23T10:30:00Z"),
        (ClaimStatus::Released, "2026-06-23T10:30:00Z"),
        (ClaimStatus::Lost, "2026-06-23T10:30:00Z"),
        (ClaimStatus::Active, "2026-06-23T09:30:00Z"),
    ];
    for path in [AcquirePath::Legacy, AcquirePath::Mutation] {
        for (status, expires_at) in &cases {
            let client = FakeIssueClient::new();
            let mut collision = claim(
                "colliding-claim-id",
                "host-b/session-b",
                "2026-06-23T09:00:00Z",
                expires_at,
            );
            collision.comment_id = Some(CommentId(9));
            collision.status = status.clone();
            client.seed(snapshot(vec![comment(9, &collision)]));
            let requested = claim(
                "colliding-claim-id",
                "host-a/session-a",
                "2026-06-23T10:05:00Z",
                "2026-06-23T10:35:00Z",
            );

            let outcome = acquire_via(path, &client, requested, "2026-06-23T10:06:00Z");

            assert!(matches!(
                outcome,
                ClaimAcquireOutcome::Blocked(ref blocking)
                    if blocking.comment_id == Some(CommentId(9))
                        && blocking.claim_id == "colliding-claim-id"
                        && blocking.owner == "host-b/session-b"
            ));
            assert_eq!(
                client.call_log(),
                vec!["fetch:#42"],
                "foreign identity collision must not create or patch"
            );
            assert_eq!(stored_claim(&client, 9), collision);
        }
    }
}

#[test]
fn cross_issue_collision_is_fail_closed_for_every_status_and_expiry() {
    let cases = [
        (ClaimStatus::Active, "2026-06-23T10:30:00Z"),
        (ClaimStatus::Released, "2026-06-23T10:30:00Z"),
        (ClaimStatus::Lost, "2026-06-23T10:30:00Z"),
        (ClaimStatus::Active, "2026-06-23T09:30:00Z"),
    ];
    for path in [AcquirePath::Legacy, AcquirePath::Mutation] {
        for (status, expires_at) in &cases {
            let client = FakeIssueClient::new();
            let mut collision = claim(
                "colliding-claim-id",
                "host-a/session-a",
                "2026-06-23T09:00:00Z",
                expires_at,
            );
            collision.comment_id = Some(CommentId(9));
            collision.status = status.clone();
            collision.issue_number = 43;
            client.seed(snapshot(vec![comment(9, &collision)]));
            let requested = claim(
                "colliding-claim-id",
                "host-a/session-a",
                "2026-06-23T10:05:00Z",
                "2026-06-23T10:35:00Z",
            );

            let outcome = acquire_via(path, &client, requested, "2026-06-23T10:06:00Z");

            assert!(matches!(
                outcome,
                ClaimAcquireOutcome::Blocked(ref blocking)
                    if blocking.comment_id == Some(CommentId(9))
                        && blocking.claim_id == "colliding-claim-id"
                        && blocking.owner == "host-a/session-a"
                        && blocking.issue_number == 43
            ));
            assert_eq!(
                client.call_log(),
                vec!["fetch:#42"],
                "cross-Issue identity collision must not create or patch"
            );
            assert_eq!(stored_claim(&client, 9), collision);
        }
    }
}

#[test]
fn exact_terminal_or_expired_identity_replay_refreshes_the_same_comment() {
    let cases = [
        (ClaimStatus::Released, "2026-06-23T10:30:00Z"),
        (ClaimStatus::Lost, "2026-06-23T10:30:00Z"),
        (ClaimStatus::Active, "2026-06-23T09:30:00Z"),
    ];
    for path in [AcquirePath::Legacy, AcquirePath::Mutation] {
        for (status, expires_at) in &cases {
            let client = FakeIssueClient::new();
            let mut existing = claim(
                "stable-effect-claim",
                "host-a/session-a",
                "2026-06-23T09:00:00Z",
                expires_at,
            );
            existing.status = status.clone();
            client.seed(snapshot(vec![comment(9, &existing)]));
            let requested = claim(
                "stable-effect-claim",
                "host-a/session-a",
                "2026-06-23T10:05:00Z",
                "2026-06-23T10:35:00Z",
            );

            let outcome = acquire_via(path, &client, requested.clone(), "2026-06-23T10:06:00Z");

            assert!(matches!(
                outcome,
                ClaimAcquireOutcome::Acquired(ref acquired)
                    if acquired.comment_id == Some(CommentId(9))
                        && acquired.claim_id == "stable-effect-claim"
                        && acquired.owner == "host-a/session-a"
                        && acquired.status == ClaimStatus::Active
            ));
            assert_eq!(client.comments(IssueNumber(42)).len(), 1);
            assert_eq!(
                client.call_log(),
                vec!["fetch:#42", "patch_comment:comment:9", "fetch:#42"]
            );
            let refreshed = stored_claim(&client, 9);
            assert_eq!(refreshed.status, ClaimStatus::Active);
            assert_eq!(refreshed.heartbeat_at, requested.heartbeat_at);
            assert_eq!(refreshed.expires_at, requested.expires_at);
        }
    }
}

#[test]
fn legacy_same_owner_different_id_does_not_patch_or_enable_a_concurrent_collision() {
    let inner = FakeIssueClient::new();
    let mut existing = claim(
        "claim-old",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    existing.comment_id = Some(CommentId(9));
    inner.seed(snapshot(vec![comment(9, &existing)]));
    let foreign = claim(
        "claim-new",
        "host-b/session-b",
        "2026-06-23T10:06:00Z",
        "2026-06-23T10:36:00Z",
    );
    let client = PatchFaultClient::inject_after_first_patch(inner, IssueNumber(42), &foreign);
    let requested = claim(
        "claim-new",
        "host-a/session-a",
        "2026-06-23T10:05:00Z",
        "2026-06-23T10:35:00Z",
    );

    let outcome = acquire_claim(&client, IssueNumber(42), requested, "2026-06-23T10:07:00Z")
        .expect("different logical identity is blocked");

    assert!(matches!(
        outcome,
        ClaimAcquireOutcome::Blocked(ref winner)
            if winner.claim_id == "claim-old" && winner.owner == "host-a/session-a"
    ));
    assert_eq!(stored_claim(&client.inner, 9), existing);
    assert_eq!(client.patch_attempts.load(Ordering::SeqCst), 0);
    assert_eq!(client.inner.call_log(), vec!["fetch:#42"]);
}

#[test]
fn both_acquire_paths_reject_a_non_winning_foreign_identity_collision() {
    for path in [AcquirePath::Legacy, AcquirePath::Mutation] {
        let client = FakeIssueClient::new();
        let own = claim(
            "colliding-claim-id",
            "host-a/session-a",
            "2026-06-23T10:00:00Z",
            "2026-06-23T10:30:00Z",
        );
        let foreign = claim(
            "colliding-claim-id",
            "host-b/session-b",
            "2026-06-23T10:01:00Z",
            "2026-06-23T10:31:00Z",
        );
        client.seed(snapshot(vec![comment(9, &own), comment(10, &foreign)]));

        let outcome = acquire_via(path, &client, own, "2026-06-23T10:02:00Z");

        assert!(matches!(
            outcome,
            ClaimAcquireOutcome::Lost { ref winning_claim, .. }
                if winning_claim.owner == "host-b/session-b"
        ));
        assert_eq!(stored_claim(&client, 9).status, ClaimStatus::Lost);
        assert_eq!(stored_claim(&client, 10).status, ClaimStatus::Active);
    }
}

#[test]
fn mutation_own_winner_rejects_a_non_winning_cross_issue_collision() {
    let client = FakeIssueClient::new();
    let own = claim(
        "colliding-claim-id",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    let mut wrong_issue = claim(
        "colliding-claim-id",
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
        "2026-06-23T10:31:00Z",
    );
    wrong_issue.issue_number = 43;
    client.seed(snapshot(vec![comment(9, &own), comment(10, &wrong_issue)]));

    let outcome = acquire_claim_mutation(
        &client,
        IssueNumber(42),
        own.clone(),
        "2026-06-23T10:02:00Z",
    )
    .expect("cross-Issue collision is an ordinary resolved loss");

    assert!(matches!(
        outcome,
        ClaimAcquireOutcome::Lost { ref winning_claim, .. }
            if winning_claim.issue_number == 43
    ));
    assert_eq!(stored_claim(&client, 9).status, ClaimStatus::Lost);
    assert_eq!(stored_claim(&client, 10).status, ClaimStatus::Active);
}

#[test]
fn both_acquire_paths_terminalize_every_non_winning_own_duplicate() {
    for path in [AcquirePath::Legacy, AcquirePath::Mutation] {
        let client = FakeIssueClient::new();
        let first = claim(
            "stable-effect-claim",
            "host-a/session-a",
            "2026-06-23T10:00:00Z",
            "2026-06-23T10:30:00Z",
        );
        let second = claim(
            "stable-effect-claim",
            "host-a/session-a",
            "2026-06-23T10:01:00Z",
            "2026-06-23T10:31:00Z",
        );
        client.seed(snapshot(vec![comment(9, &first), comment(10, &second)]));

        let outcome = acquire_via(path, &client, first, "2026-06-23T10:02:00Z");

        assert!(matches!(
            outcome,
            ClaimAcquireOutcome::Acquired(ref acquired)
                if acquired.comment_id == Some(CommentId(9))
        ));
        assert_eq!(stored_claim(&client, 9).status, ClaimStatus::Active);
        assert_eq!(stored_claim(&client, 10).status, ClaimStatus::Lost);
    }
}

#[test]
fn mutation_loss_terminalizes_every_active_own_duplicate() {
    let client = FakeIssueClient::new();
    let winner = claim(
        "foreign-claim",
        "host-b/session-b",
        "2026-06-23T09:59:00Z",
        "2026-06-23T10:29:00Z",
    );
    let first = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    let second = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:01:00Z",
        "2026-06-23T10:31:00Z",
    );
    client.seed(snapshot(vec![
        comment(8, &winner),
        comment(9, &first),
        comment(10, &second),
    ]));

    let outcome = acquire_claim_mutation(&client, IssueNumber(42), first, "2026-06-23T10:02:00Z")
        .expect("foreign winner is a resolved loss");

    assert!(matches!(outcome, ClaimAcquireOutcome::Lost { .. }));
    assert_eq!(stored_claim(&client, 9).status, ClaimStatus::Lost);
    assert_eq!(stored_claim(&client, 10).status, ClaimStatus::Lost);
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
        "host-a/old-process",
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
fn foreign_owner_cannot_acquire_an_existing_claim_id() {
    let client = FakeIssueClient::new();
    let foreign = claim(
        "colliding-claim-id",
        "host-b/session-b",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    client.seed(snapshot(vec![comment(9, &foreign)]));
    let requested = claim(
        "colliding-claim-id",
        "host-a/session-a",
        "2026-06-23T10:05:00Z",
        "2026-06-23T10:35:00Z",
    );

    let outcome =
        acquire_claim_mutation(&client, IssueNumber(42), requested, "2026-06-23T10:06:00Z")
            .expect("foreign identity collision is a resolved blocked outcome");

    assert!(matches!(
        outcome,
        ClaimAcquireOutcome::Blocked(ref blocking)
            if blocking.claim_id == "colliding-claim-id"
                && blocking.owner == "host-b/session-b"
                && blocking.issue_number == 42
    ));
    assert_eq!(client.call_log(), vec!["fetch:#42"]);
}

#[test]
fn claim_readback_rejects_same_id_and_owner_for_another_issue() {
    let client = FakeIssueClient::new();
    let mut wrong_issue = claim(
        "colliding-claim-id",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    wrong_issue.issue_number = 43;
    client.seed(snapshot(vec![comment(9, &wrong_issue)]));
    let requested = claim(
        "colliding-claim-id",
        "host-a/session-a",
        "2026-06-23T10:05:00Z",
        "2026-06-23T10:35:00Z",
    );

    let outcome =
        acquire_claim_mutation(&client, IssueNumber(42), requested, "2026-06-23T10:06:00Z")
            .expect("cross-Issue identity collision is a resolved blocked outcome");

    assert!(matches!(
        outcome,
        ClaimAcquireOutcome::Blocked(ref blocking)
            if blocking.claim_id == "colliding-claim-id"
                && blocking.owner == "host-a/session-a"
                && blocking.issue_number == 43
    ));
    assert_eq!(client.call_log(), vec!["fetch:#42"]);
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
        "host-a/old-process",
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
        "host-a/old-process",
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

    let first = release_claim(
        &client,
        IssueNumber(42),
        "stable-effect-claim",
        "host-a/session-a",
    )
    .expect("first release succeeds");
    assert!(matches!(first, ClaimReleaseOutcome::Released(_)));

    let second = release_claim(
        &client,
        IssueNumber(42),
        "stable-effect-claim",
        "host-a/session-a",
    )
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

#[test]
fn release_patches_only_the_exact_issue_claim_and_owner_identity() {
    let client = FakeIssueClient::new();
    let foreign = claim(
        "stable-effect-claim",
        "host-b/session-b",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    let owned = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:01Z",
        "2026-06-23T10:30:01Z",
    );
    client.seed(snapshot(vec![comment(9, &foreign), comment(10, &owned)]));

    let outcome = release_claim(
        &client,
        IssueNumber(42),
        "stable-effect-claim",
        "host-a/session-a",
    )
    .expect("exact logical claim releases");

    assert!(matches!(outcome, ClaimReleaseOutcome::Released(_)));
    assert_eq!(stored_claim(&client, 9).status, ClaimStatus::Active);
    assert_eq!(stored_claim(&client, 10).status, ClaimStatus::Released);
    assert_eq!(
        client.call_log(),
        vec!["fetch:#42", "patch_comment:comment:10"],
        "a foreign same-id claim must never be patched"
    );
}

#[test]
fn mutation_release_patches_only_the_exact_issue_claim_and_owner_identity() {
    let client = FakeIssueClient::new();
    let foreign = claim(
        "stable-effect-claim",
        "host-b/session-b",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    let owned = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:01Z",
        "2026-06-23T10:30:01Z",
    );
    client.seed(snapshot(vec![comment(9, &foreign), comment(10, &owned)]));

    let outcome = release_claim_mutation(
        &client,
        IssueNumber(42),
        "stable-effect-claim",
        "host-a/session-a",
    )
    .expect("exact logical claim reconciles");

    assert!(matches!(outcome, ClaimReleaseOutcome::Released(_)));
    assert_eq!(stored_claim(&client, 9).status, ClaimStatus::Active);
    assert_eq!(stored_claim(&client, 10).status, ClaimStatus::Released);
    assert_eq!(
        client.call_log(),
        vec!["fetch:#42", "patch_comment:comment:10"],
        "a foreign same-id claim must never be patched"
    );
}

#[test]
fn ownerless_legacy_release_is_rejected_without_patching_a_same_id_claim() {
    let client = FakeIssueClient::new();
    let foreign = claim(
        "stable-effect-claim",
        "host-b/session-b",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    client.seed(snapshot(vec![comment(9, &foreign)]));

    let error = release_claim(&client, IssueNumber(42), "stable-effect-claim", "")
        .expect_err("an ownerless legacy release must fail closed");

    assert!(matches!(
        error,
        ApiError::Unexpected(message) if message.contains("owner")
    ));
    assert_eq!(stored_claim(&client, 9).status, ClaimStatus::Active);
    assert!(
        client.call_log().is_empty(),
        "invalid owner identity must fail before remote readback or patch"
    );
}

#[test]
fn ownerless_legacy_mutation_release_is_rejected_without_patching_a_same_id_claim() {
    let client = FakeIssueClient::new();
    let foreign = claim(
        "stable-effect-claim",
        "host-b/session-b",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    client.seed(snapshot(vec![comment(9, &foreign)]));

    let error = release_claim_mutation(&client, IssueNumber(42), "stable-effect-claim", "")
        .expect_err("an ownerless legacy release must fail closed");

    assert!(matches!(
        error,
        OwnerMutationError::PreSubmit(ApiError::Unexpected(message))
            if message.contains("owner")
    ));
    assert_eq!(stored_claim(&client, 9).status, ClaimStatus::Active);
    assert!(
        client.call_log().is_empty(),
        "invalid owner identity must fail before remote readback or patch"
    );
}

#[test]
fn release_terminalizes_every_active_duplicate_for_the_logical_claim() {
    let client = FakeIssueClient::new();
    let first = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    let second = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:01Z",
        "2026-06-23T10:30:01Z",
    );
    client.seed(snapshot(vec![comment(9, &first), comment(10, &second)]));

    let outcome = release_claim(
        &client,
        IssueNumber(42),
        "stable-effect-claim",
        "host-a/session-a",
    )
    .expect("duplicate logical claim releases");

    assert!(matches!(outcome, ClaimReleaseOutcome::Released(_)));
    let stored = client.comments(IssueNumber(42));
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|comment| {
        parse_claim_comment(Some(comment.id), &comment.body)
            .is_ok_and(|claim| claim.status == ClaimStatus::Released)
    }));
    assert_eq!(
        client.call_log(),
        vec![
            "fetch:#42",
            "patch_comment:comment:9",
            "patch_comment:comment:10",
        ]
    );
}

#[test]
fn mutation_reconcile_terminalizes_every_active_duplicate_for_the_logical_claim() {
    let client = FakeIssueClient::new();
    let first = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    let second = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:01Z",
        "2026-06-23T10:30:01Z",
    );
    client.seed(snapshot(vec![comment(9, &first), comment(10, &second)]));

    let outcome = release_claim_mutation(
        &client,
        IssueNumber(42),
        "stable-effect-claim",
        "host-a/session-a",
    )
    .expect("duplicate logical claim reconciles");

    assert!(matches!(outcome, ClaimReleaseOutcome::Released(_)));
    let stored = client.comments(IssueNumber(42));
    assert_eq!(stored.len(), 2);
    assert!(stored.iter().all(|comment| {
        parse_claim_comment(Some(comment.id), &comment.body)
            .is_ok_and(|claim| claim.status == ClaimStatus::Released)
    }));
    assert_eq!(
        client.call_log(),
        vec![
            "fetch:#42",
            "patch_comment:comment:9",
            "patch_comment:comment:10",
        ]
    );
}

#[test]
fn mutation_release_promotes_partial_presubmit_failure_and_replay_converges() {
    let inner = FakeIssueClient::new();
    let first = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:00Z",
        "2026-06-23T10:30:00Z",
    );
    let second = claim(
        "stable-effect-claim",
        "host-a/session-a",
        "2026-06-23T10:00:01Z",
        "2026-06-23T10:30:01Z",
    );
    inner.seed(snapshot(vec![comment(9, &first), comment(10, &second)]));
    let client = PatchFaultClient::fail_at(inner, 2);

    let error = release_claim_mutation(
        &client,
        IssueNumber(42),
        "stable-effect-claim",
        "host-a/session-a",
    )
    .expect_err("a later pre-submit failure follows an earlier submitted patch");

    assert!(matches!(
        error,
        OwnerMutationError::RemoteOutcomeUnknown(ApiError::Network(message))
            if message.contains("attempt 2")
    ));
    assert_eq!(stored_claim(&client.inner, 9).status, ClaimStatus::Released);
    assert_eq!(stored_claim(&client.inner, 10).status, ClaimStatus::Active);

    let replay = release_claim_mutation(
        &client,
        IssueNumber(42),
        "stable-effect-claim",
        "host-a/session-a",
    )
    .expect("replay terminalizes the remaining duplicate");
    assert!(matches!(replay, ClaimReleaseOutcome::Released(_)));
    assert_eq!(stored_claim(&client.inner, 9).status, ClaimStatus::Released);
    assert_eq!(
        stored_claim(&client.inner, 10).status,
        ClaimStatus::Released
    );
}
