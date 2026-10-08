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

use std::{
    collections::BTreeMap,
    path::Path,
    time::{Duration, Instant},
};

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

/// Shared delivery check for merge settlements. A conflict is advisory:
/// an equivalent change may have landed in a
/// different PR. Unknown delivery must remain visible as a warning, too.
/// `stage` names the check in the warning (close 前検査 / merge 後検査).
pub fn issue_delivery_warning(repo_path: &Path, issue_number: u64, stage: &str) -> Option<String> {
    let repo_path = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let check = || -> Result<Vec<String>, String> {
        // A successful limited fetch cannot prove an absent owner ref was
        // pruned. Do not rewrite the caller's remote configuration here.
        let fetchspec = gwt_core::process::run_git_logged(
            &["config", "--get-all", "remote.origin.fetch"],
            Some(&repo_path),
        )
        .map_err(|error| error.to_string())?;
        let fetchspec = String::from_utf8_lossy(&fetchspec.stdout);
        if !matches!(
            fetchspec.trim(),
            "+refs/heads/*:refs/remotes/origin/*" | "refs/heads/*:refs/remotes/origin/*"
        ) {
            return Err(
                "origin の fetch 設定では全 owner branch の確認を保証できません".to_string(),
            );
        }
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            Instant::now() + SETTLEMENT_TOTAL_TIMEOUT,
        );
        gwt_git::worktree::WorktreeManager::new(&repo_path)
            .fetch_origin()
            .map_err(|error| error.to_string())?;
        let base_ref = "refs/remotes/origin/develop";
        let local_ref = format!("refs/heads/work/issue-{issue_number}");
        let remote_ref = format!("refs/remotes/origin/work/issue-{issue_number}");
        let output = gwt_core::process::run_git_logged(
            &[
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                base_ref,
                &local_ref,
                &remote_ref,
            ],
            Some(&repo_path),
        )
        .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let refs: BTreeMap<_, _> = stdout
            .lines()
            .filter_map(|line| line.split_once(' '))
            .collect();
        let base = refs
            .get(base_ref)
            .ok_or("origin/develop の commit を確認できません")?;
        let mut changes = Vec::new();
        for branch in [&local_ref, &remote_ref] {
            let Some(head) = refs.get(branch.as_str()) else {
                continue;
            };
            // Pin both OIDs so the warning describes the exact projection,
            // even if another worktree fetches while this check runs.
            if gwt_git::pr_status::branch_has_non_gwt_changes(&repo_path, base, head)? {
                changes.push(format!("- `{branch}`: head `{head}`, develop `{base}`"));
            }
        }
        Ok(changes)
    };
    let detail = match check() {
        Ok(changes) if changes.is_empty() => return None,
        Ok(changes) => format!(
            "merge projection に `.gwt/` 以外の差分が残っています。\n\n{}\n\n競合や別 PR の等価実装でも差分が出るため、配信漏れとは断定していません。必要な変更の配信・移管・破棄の判断を確認してください。",
            changes.join("\n"),
        ),
        Err(error) => format!("配信状態を確認できませんでした: {error}\n\n必要な変更が develop に配信済みか確認してください。"),
    };
    Some(format!("<!-- gwt-issue-close-delivery-warning v1 -->\n\n警告: Issue #{issue_number} の{stage}で、{detail}"))
}

/// Run one settlement against GitHub. Idempotent per delivery: a retry that
/// finds its marker comment skips the comment, and a close that finds the
/// Issue already closed reports `already_closed` instead of mutating.
pub fn settle_merged_issue<C: IssueClient + OwnerRepositoryClient>(
    client: &C,
    repository: &RepositoryIdentity,
    repo_path: &Path,
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
    let already_commented = settlement_already_commented(
        snapshot
            .comments
            .iter()
            .map(|comment| comment.body.as_str()),
        &marker,
    );
    // A close re-checks on every attempt; other settlements check once per
    // merge so an unpushed owner commit is surfaced before it is stranded
    // (Issue #4615).
    if wants_close || !already_commented {
        let stage = if wants_close {
            "close 前検査"
        } else {
            "merge 後検査"
        };
        if let Some(warning) = issue_delivery_warning(repo_path, issue_number, stage) {
            if !snapshot
                .comments
                .iter()
                .any(|comment| comment.body == warning)
            {
                // A previous settlement marker does not certify today's head.
                // If the warning cannot be saved, do not submit the close.
                client.create_comment_mutation(number, &warning)?;
            }
        }
    }
    let commented = if already_commented {
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
        let repo = delivery_repo();
        seed_open_issue(&client, 42, vec![]);
        let outcome = settle_merged_issue(
            &client,
            &repository(),
            repo.path(),
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
        let repo = delivery_repo();
        let marker = settlement_marker(7, Some("abc123"));
        seed_open_issue(&client, 42, vec![&format!("{marker}\n\nearlier attempt")]);
        let outcome = settle_merged_issue(
            &client,
            &repository(),
            repo.path(),
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
        let repo = delivery_repo();
        seed_open_issue(&client, 42, vec![]);
        seed_open_issue(&client, 43, vec![]);
        let unmet = settle_merged_issue(
            &client,
            &repository(),
            repo.path(),
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
            repo.path(),
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
        let repo = delivery_repo();
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
            repo.path(),
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
        let repo = delivery_repo();
        let error = settle_merged_issue(
            &client,
            &repository(),
            repo.path(),
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
    fn git(repo: &std::path::Path, args: &[&str]) -> String {
        let output = gwt_core::process::run_git_logged(args, Some(repo)).expect("git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("utf8")
            .trim()
            .to_string()
    }

    fn delivery_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().expect("repo");
        let path = repo.path();
        git(path, &["init", "-b", "develop"]);
        git(path, &["config", "user.name", "Test"]);
        git(path, &["config", "user.email", "test@example.com"]);
        git(path, &["commit", "--allow-empty", "-m", "base"]);
        git(path, &["clone", "--bare", ".", "origin.git"]);
        git(path, &["remote", "add", "origin", "origin.git"]);
        repo
    }

    fn push_unlanded_commit(repo: &std::path::Path, file: &str) -> String {
        let base = git(repo, &["rev-parse", "HEAD"]);
        let path = repo.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "unlanded").unwrap();
        git(repo, &["add", file]);
        git(repo, &["commit", "-m", "unlanded"]);
        let head = git(repo, &["rev-parse", "HEAD"]);
        git(repo, &["push", "origin", "HEAD:refs/heads/work/issue-42"]);
        git(repo, &["update-ref", "refs/heads/develop", &base]);
        // A close must refresh tracking refs, not treat their absence as delivery.
        git(
            repo,
            &["update-ref", "-d", "refs/remotes/origin/work/issue-42"],
        );
        head
    }

    #[test]
    fn close_rechecks_source_delivery_even_after_its_settlement_comment() {
        let repo = delivery_repo();
        let head = push_unlanded_commit(repo.path(), "source.rs");
        let client = FakeIssueClient::new();
        let marker = settlement_marker(7, Some("abc123"));
        seed_open_issue(&client, 42, vec![&marker]);
        let outcome = settle_merged_issue(
            &client,
            &repository(),
            repo.path(),
            42,
            7,
            Some("abc123"),
            &MergedIssueSettlementAction::Close { delegated: false },
        )
        .expect("warned then closed");
        assert!(outcome.closed);
        let comments = client.comments(IssueNumber(42));
        assert_eq!(
            comments.len(),
            2,
            "existing settlement must not suppress the warning"
        );
        assert!(comments[1]
            .body
            .contains("gwt-issue-close-delivery-warning"));
        assert!(comments[1].body.contains("work/issue-42"));
        assert!(comments[1].body.contains(&head));

        // The same source also needs a warning when only an unpushed local
        // owner ref remains; a remote-only check would lose this work.
        git(
            repo.path(),
            &[
                "--git-dir=origin.git",
                "update-ref",
                "-d",
                "refs/heads/work/issue-42",
            ],
        );
        git(
            repo.path(),
            &["update-ref", "refs/heads/work/issue-42", &head],
        );
        let warning =
            issue_delivery_warning(repo.path(), 42, "close 前検査").expect("local source warning");
        assert!(warning.contains("`refs/heads/work/issue-42`"));
        assert!(!warning.contains("refs/remotes/origin/work/issue-42"));
    }

    #[test]
    fn unmet_settlement_warns_once_about_an_unlanded_local_commit() {
        // Issue #4615 AC-4: #4614 merged with unmet AC while its fix commit
        // existed only on the local owner branch. Every merge settlement, not
        // only a close, must surface that the commit can be stranded.
        let repo = delivery_repo();
        let head = push_unlanded_commit(repo.path(), "source.rs");
        git(
            repo.path(),
            &[
                "--git-dir=origin.git",
                "update-ref",
                "-d",
                "refs/heads/work/issue-42",
            ],
        );
        git(
            repo.path(),
            &["update-ref", "refs/heads/work/issue-42", &head],
        );
        let client = FakeIssueClient::new();
        seed_open_issue(&client, 42, vec![]);
        let action = MergedIssueSettlementAction::UnmetAcceptance {
            unmet: vec!["AC-1".to_string()],
        };
        let settle = || {
            settle_merged_issue(
                &client,
                &repository(),
                repo.path(),
                42,
                7,
                Some("abc123"),
                &action,
            )
            .expect("settled")
        };
        settle();
        let comments = client.comments(IssueNumber(42));
        assert_eq!(comments.len(), 2, "warning + settlement comment");
        assert!(
            comments[0].body.contains("merge 後検査"),
            "{}",
            comments[0].body
        );
        assert!(comments[0].body.contains("`refs/heads/work/issue-42`"));
        assert!(comments[0].body.contains(&head));

        // A retry after the settlement landed must not repeat the warning.
        settle();
        assert_eq!(client.comments(IssueNumber(42)).len(), 2);
    }

    #[test]
    fn close_ignores_bookkeeping_but_warns_when_remote_coverage_is_unknown() {
        let repo = delivery_repo();
        push_unlanded_commit(repo.path(), ".gwt/state");
        assert!(issue_delivery_warning(repo.path(), 42, "close 前検査").is_none());
        git(
            repo.path(),
            &[
                "config",
                "remote.origin.fetch",
                "+refs/heads/develop:refs/remotes/origin/develop",
            ],
        );
        assert!(issue_delivery_warning(repo.path(), 42, "close 前検査").is_some());
    }

    #[test]
    fn close_is_not_submitted_when_delivery_warning_cannot_be_saved() {
        let repo = delivery_repo();
        push_unlanded_commit(repo.path(), "source.rs");
        let client = FakeIssueClient::new();
        let marker = settlement_marker(7, Some("abc123"));
        seed_open_issue(&client, 42, vec![&marker]);
        client.fail_create_comment_after(0);
        assert!(settle_merged_issue(
            &client,
            &repository(),
            repo.path(),
            42,
            7,
            Some("abc123"),
            &MergedIssueSettlementAction::Close { delegated: false },
        )
        .is_err());
        assert_eq!(client.owner_mutation_count(), 0);
    }
}
