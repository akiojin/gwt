//! Issue #3917 — settle a delivered Issue on GitHub after its work merged.
//!
//! The Issue Monitor scan proposes one [`IssueMonitorEffectPayload::SettleMergedIssue`]
//! per merged delivery; the daemon executor runs [`settle_merged_issue`], which
//! posts the settlement comment (once, keyed by a marker) and, for
//! [`MergedIssueSettlementAction::Close`], closes the Issue with a verified
//! readback. `Closes #N` only fires on the default branch, so without this the
//! Issue stays open until a release PR merges.
//!
//! [`IssueMonitorEffectPayload::SettleMergedIssue`]: crate::IssueMonitorEffectPayload::SettleMergedIssue

use std::time::Duration;

use gwt_github::client::{
    FetchResult, IssueClient, OwnerMutationError, OwnerMutationResult, OwnerRepositoryClient,
    RepositoryIdentity, ResolutionDeadline,
};
use gwt_github::{IssueNumber, IssueState};

use crate::issue_monitor::MergedIssueSettlementAction;

/// Marker prefix of every settlement comment. The full marker carries the PR
/// number and merge SHA so a retried effect can prove its comment landed.
pub const SETTLEMENT_MARKER_PREFIX: &str = "<!-- gwt-merged-issue-settlement v1";

/// Connect timeout cap for the settlement mutation.
const SETTLEMENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Total budget for the close + readback of one settlement.
const SETTLEMENT_TOTAL_TIMEOUT: Duration = Duration::from_secs(30);

/// What the executor actually did on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergedIssueSettlementOutcome {
    /// The settlement comment was posted by this attempt.
    pub commented: bool,
    /// The Issue was closed by this attempt.
    pub closed: bool,
    /// The Issue was already closed when the attempt read it back.
    pub already_closed: bool,
}

/// The idempotency marker for one delivery.
pub fn settlement_marker(pr_number: u64, merge_sha: Option<&str>) -> String {
    format!(
        "{SETTLEMENT_MARKER_PREFIX} pr={pr_number} sha={} -->",
        merge_sha.unwrap_or("unknown")
    )
}

/// Whether one of `comment_bodies` already carries `marker`.
pub fn settlement_already_commented<'a>(
    comment_bodies: impl IntoIterator<Item = &'a str>,
    marker: &str,
) -> bool {
    comment_bodies.into_iter().any(|body| body.contains(marker))
}

/// Render the settlement comment. Narrative text is Japanese to match the
/// project's Issue conventions; the marker keeps it machine-recognizable.
pub fn render_settlement_comment(
    issue_number: u64,
    pr_number: u64,
    merge_sha: Option<&str>,
    action: &MergedIssueSettlementAction,
) -> String {
    let sha = merge_sha.unwrap_or("unknown");
    let mut body = String::new();
    body.push_str(&settlement_marker(pr_number, merge_sha));
    body.push_str("\n\n");
    match action {
        MergedIssueSettlementAction::Close { delegated } => {
            body.push_str(&format!(
                "PR #{pr_number}（merge commit `{sha}`）が develop に merge されたため、gwt Issue Monitor が Issue #{issue_number} を close します。\n"
            ));
            if *delegated {
                body.push_str("\n残 AC は別 Issue に委譲済みの記録を確認しました。\n");
            } else {
                body.push_str("\n受け入れ基準はすべて `[x]` です。\n");
            }
        }
        MergedIssueSettlementAction::AwaitClose { unmet } => {
            body.push_str(&format!(
                "merge 済み・close 待ち: PR #{pr_number}（merge commit `{sha}`）が merge されました。auto-close が off のため Issue は open のままです。\n"
            ));
            if !unmet.is_empty() {
                body.push_str(&format!("\n未達 AC: {}\n", unmet.join(", ")));
            }
        }
        MergedIssueSettlementAction::UnmetAcceptance { unmet } => {
            body.push_str(&format!(
                "merge 済み・未達 AC あり: PR #{pr_number}（merge commit `{sha}`）は merge されましたが、受け入れ基準が未達のため close しません。needs_human として人間の判断を待ちます。\n"
            ));
            if unmet.is_empty() {
                body.push_str("\n受け入れ基準ブロック（`- [ ] AC-N:`）が見つかりません。\n");
            } else {
                body.push_str(&format!("\n未達 AC: {}\n", unmet.join(", ")));
            }
            body.push_str(
                "\n残 AC を別 Issue に委譲する場合は、PR 本文または Issue コメントに「残 AC は別 Issue に委譲」と記録してください。\n",
            );
        }
    }
    body.push_str("\nManaged by gwt Issue Monitor.\n");
    body
}

/// Run one settlement against GitHub. Idempotent per delivery: a retry that
/// finds its marker comment skips the comment, and a close that finds the
/// Issue already closed reports `already_closed` instead of mutating.
pub fn settle_merged_issue<C: IssueClient + OwnerRepositoryClient>(
    client: &C,
    repository: &RepositoryIdentity,
    issue_number: u64,
    pr_number: u64,
    merge_sha: Option<&str>,
    action: &MergedIssueSettlementAction,
) -> OwnerMutationResult<MergedIssueSettlementOutcome> {
    let number = IssueNumber(issue_number);
    let snapshot = match client.fetch(number, None) {
        Ok(FetchResult::Updated(snapshot)) => snapshot,
        Ok(FetchResult::NotModified) => {
            return Err(OwnerMutationError::PreSubmit(
                gwt_github::ApiError::Unexpected(
                    "unconditional issue fetch reported not modified".to_string(),
                ),
            ))
        }
        Err(error) => return Err(OwnerMutationError::PreSubmit(error)),
    };
    let wants_close = matches!(action, MergedIssueSettlementAction::Close { .. });
    if wants_close && snapshot.state == IssueState::Closed {
        return Ok(MergedIssueSettlementOutcome {
            commented: false,
            closed: false,
            already_closed: true,
        });
    }
    let marker = settlement_marker(pr_number, merge_sha);
    let commented = if settlement_already_commented(
        snapshot
            .comments
            .iter()
            .map(|comment| comment.body.as_str()),
        &marker,
    ) {
        false
    } else {
        client.create_comment_mutation(
            number,
            &render_settlement_comment(issue_number, pr_number, merge_sha, action),
        )?;
        true
    };
    let closed = if wants_close {
        let deadline =
            ResolutionDeadline::new(SETTLEMENT_CONNECT_TIMEOUT, SETTLEMENT_TOTAL_TIMEOUT);
        client.close_issue_verified(repository, number, &deadline)?;
        true
    } else {
        false
    };
    Ok(MergedIssueSettlementOutcome {
        commented,
        closed,
        already_closed: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use gwt_github::client::fake::FakeIssueClient;
    use gwt_github::client::{
        CommentSnapshot, IssueSnapshot, RepositoryIssue, RepositoryIssueKind, UpdatedAt,
    };
    use gwt_github::CommentId;

    fn repository() -> RepositoryIdentity {
        RepositoryIdentity::new("example", "repo")
    }

    fn seed_open_issue(client: &FakeIssueClient, number: u64, comments: Vec<&str>) {
        client.seed(IssueSnapshot {
            number: IssueNumber(number),
            title: format!("Issue {number}"),
            body: "## Acceptance Criteria\n- [x] AC-1: done\n".to_string(),
            labels: vec![],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("2026-09-01T00:00:00Z"),
            comments: comments
                .into_iter()
                .enumerate()
                .map(|(index, body)| CommentSnapshot {
                    id: CommentId(index as u64 + 1),
                    body: body.to_string(),
                    updated_at: UpdatedAt::new("2026-09-01T00:00:00Z"),
                })
                .collect(),
        });
        client.seed_repository_issue(RepositoryIssue {
            repository: repository(),
            number: IssueNumber(number),
            title: format!("Issue {number}"),
            body: String::new(),
            labels: vec![],
            state: IssueState::Open,
            kind: RepositoryIssueKind::Plain,
            updated_at: UpdatedAt::new("2026-09-01T00:00:00Z"),
        });
    }

    #[test]
    fn close_settlement_comments_then_closes_with_readback() {
        // Issue #3917 AC-1: close + comment carrying merge SHA and PR number.
        let client = FakeIssueClient::new();
        seed_open_issue(&client, 42, vec![]);
        let outcome = settle_merged_issue(
            &client,
            &repository(),
            42,
            7,
            Some("abc123"),
            &MergedIssueSettlementAction::Close { delegated: false },
        )
        .expect("settled");
        assert_eq!(
            outcome,
            MergedIssueSettlementOutcome {
                commented: true,
                closed: true,
                already_closed: false
            }
        );
        let comments = client.comments(IssueNumber(42));
        assert_eq!(comments.len(), 1);
        let body = &comments[0].body;
        assert!(
            body.contains(&settlement_marker(7, Some("abc123"))),
            "{body}"
        );
        assert!(body.contains("PR #7") && body.contains("abc123"), "{body}");
        assert!(
            client
                .owner_mutation_call_log()
                .iter()
                .any(|call| format!("{call:?}").contains("CloseIssue")),
            "the close went through the verified owner mutation"
        );
    }

    #[test]
    fn retry_skips_the_comment_when_its_marker_already_landed() {
        let client = FakeIssueClient::new();
        let marker = settlement_marker(7, Some("abc123"));
        seed_open_issue(&client, 42, vec![&format!("{marker}\n\nearlier attempt")]);
        let outcome = settle_merged_issue(
            &client,
            &repository(),
            42,
            7,
            Some("abc123"),
            &MergedIssueSettlementAction::Close { delegated: true },
        )
        .expect("settled");
        assert!(!outcome.commented, "marker present: no duplicate comment");
        assert!(outcome.closed);
        assert_eq!(client.comments(IssueNumber(42)).len(), 1);
    }

    #[test]
    fn unmet_and_await_settlements_only_comment() {
        let client = FakeIssueClient::new();
        seed_open_issue(&client, 42, vec![]);
        seed_open_issue(&client, 43, vec![]);
        let unmet = settle_merged_issue(
            &client,
            &repository(),
            42,
            7,
            None,
            &MergedIssueSettlementAction::UnmetAcceptance {
                unmet: vec!["AC-2".to_string()],
            },
        )
        .expect("commented");
        assert_eq!(
            unmet,
            MergedIssueSettlementOutcome {
                commented: true,
                closed: false,
                already_closed: false
            }
        );
        let body = &client.comments(IssueNumber(42))[0].body;
        assert!(
            body.contains("merge 済み・未達 AC あり") && body.contains("AC-2"),
            "{body}"
        );
        assert!(body.contains("sha=unknown"), "{body}");

        let awaiting = settle_merged_issue(
            &client,
            &repository(),
            43,
            8,
            Some("def456"),
            &MergedIssueSettlementAction::AwaitClose { unmet: vec![] },
        )
        .expect("commented");
        assert!(awaiting.commented && !awaiting.closed);
        let body = &client.comments(IssueNumber(43))[0].body;
        assert!(body.contains("merge 済み・close 待ち"), "{body}");
        assert!(
            client.owner_mutation_count() == 0,
            "no close mutation for comment-only settlements"
        );
    }

    #[test]
    fn already_closed_issue_is_reported_without_mutation() {
        let client = FakeIssueClient::new();
        client.seed(IssueSnapshot {
            number: IssueNumber(42),
            title: "closed".to_string(),
            body: String::new(),
            labels: vec![],
            state: IssueState::Closed,
            updated_at: UpdatedAt::new("2026-09-01T00:00:00Z"),
            comments: vec![],
        });
        let outcome = settle_merged_issue(
            &client,
            &repository(),
            42,
            7,
            Some("abc123"),
            &MergedIssueSettlementAction::Close { delegated: false },
        )
        .expect("no-op");
        assert_eq!(
            outcome,
            MergedIssueSettlementOutcome {
                commented: false,
                closed: false,
                already_closed: true
            }
        );
        assert!(client.comments(IssueNumber(42)).is_empty());
        assert_eq!(client.owner_mutation_count(), 0);
    }

    #[test]
    fn fetch_failure_is_pre_submit() {
        let client = FakeIssueClient::new();
        let error = settle_merged_issue(
            &client,
            &repository(),
            404,
            7,
            None,
            &MergedIssueSettlementAction::Close { delegated: false },
        )
        .expect_err("missing issue");
        assert!(
            matches!(error, OwnerMutationError::PreSubmit(_)),
            "{error:?}"
        );
    }
}
