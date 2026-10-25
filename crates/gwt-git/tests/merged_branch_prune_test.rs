//! Issue #3970: candidate selection and batched deletion for merged
//! `work/issue-*` remote branches.
//!
//! The candidate rules (AC-2) and the batched delete (AC-4) are exercised
//! through the public API both the Issue Monitor delivery hook and the
//! `branch.prune_merged` bulk operation call, so a divergence between the two
//! entrances would fail here.

use std::collections::{BTreeMap, HashMap};

use gwt_git::merged_branch_prune::{
    classify_prune_candidate, execute_prune_with, plan_prune, prune_merged_branches,
    PruneCandidateFacts, PruneDecision, PruneEnvironment, PruneSkipReason, PRUNE_BATCH_SIZE,
};

/// Scripted remote used by the report-level tests. Both the delivery hook and
/// the bulk operation drive the same [`prune_merged_branches`] entry point, so
/// these cover the shared behavior of both.
#[derive(Default)]
struct FakeRemote {
    merged: BTreeMap<String, u64>,
    open: HashMap<String, u64>,
    ahead: HashMap<String, u64>,
    merged_error: Option<String>,
    open_error: Option<String>,
    deleted: Vec<String>,
}

impl PruneEnvironment for FakeRemote {
    fn merged_prs(&mut self) -> Result<BTreeMap<String, u64>, String> {
        match &self.merged_error {
            Some(error) => Err(error.clone()),
            None => Ok(self.merged.clone()),
        }
    }

    fn open_prs(&mut self) -> Result<HashMap<String, u64>, String> {
        match &self.open_error {
            Some(error) => Err(error.clone()),
            None => Ok(self.open.clone()),
        }
    }

    fn unmerged_commits(&mut self, branch: &str) -> Option<u64> {
        Some(self.ahead.get(branch).copied().unwrap_or(0))
    }

    fn delete_batch(&mut self, branches: &[String]) -> Result<(), String> {
        self.deleted.extend(branches.iter().cloned());
        Ok(())
    }
}

fn facts(branch: &str) -> PruneCandidateFacts {
    PruneCandidateFacts {
        branch: branch.to_string(),
        merged_pr: Some(100),
        open_pr: None,
        unmerged_commits: Some(0),
    }
}

#[test]
fn merged_branch_with_no_unmerged_commits_is_pruned() {
    assert_eq!(
        classify_prune_candidate(&facts("work/issue-42")),
        PruneDecision::Prune { pr_number: 100 }
    );
}

#[test]
fn branch_without_merged_pr_is_kept() {
    let mut candidate = facts("work/issue-42");
    candidate.merged_pr = None;
    assert_eq!(
        classify_prune_candidate(&candidate),
        PruneDecision::Skip(PruneSkipReason::NotMerged)
    );
}

#[test]
fn branch_with_unmerged_commits_is_kept() {
    let mut candidate = facts("work/issue-42");
    candidate.unmerged_commits = Some(3);
    assert_eq!(
        classify_prune_candidate(&candidate),
        PruneDecision::Skip(PruneSkipReason::UnmergedCommits(3))
    );
}

#[test]
fn branch_whose_ahead_count_is_unknown_is_kept() {
    let mut candidate = facts("work/issue-42");
    candidate.unmerged_commits = None;
    assert_eq!(
        classify_prune_candidate(&candidate),
        PruneDecision::Skip(PruneSkipReason::AheadUnknown)
    );
}

#[test]
fn branch_with_an_open_pr_is_kept_even_when_an_older_pr_merged() {
    let mut candidate = facts("work/issue-42");
    candidate.open_pr = Some(101);
    assert_eq!(
        classify_prune_candidate(&candidate),
        PruneDecision::Skip(PruneSkipReason::OpenPr(101))
    );
}

#[test]
fn protected_branches_are_never_pruned() {
    for branch in ["develop", "main", "master", "release/9.9.0"] {
        let candidate = facts(branch);
        assert_eq!(
            classify_prune_candidate(&candidate),
            PruneDecision::Skip(PruneSkipReason::Protected),
            "{branch} must stay protected even with a merged PR and zero unmerged commits"
        );
    }
}

#[test]
fn plan_splits_candidates_into_prune_and_skip() {
    let mut open = facts("work/issue-2");
    open.open_pr = Some(7);
    let plan = plan_prune(&[facts("work/issue-1"), open, facts("develop")]);
    assert_eq!(
        plan.prune
            .iter()
            .map(|target| target.branch.as_str())
            .collect::<Vec<_>>(),
        vec!["work/issue-1"]
    );
    assert_eq!(
        plan.skip
            .iter()
            .map(|skipped| (skipped.branch.as_str(), skipped.reason.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("work/issue-2", PruneSkipReason::OpenPr(7)),
            ("develop", PruneSkipReason::Protected),
        ]
    );
}

#[test]
fn deletion_is_batched_by_refspec_group() {
    let branches: Vec<String> = (0..32).map(|n| format!("work/issue-{n}")).collect();
    let mut batches: Vec<usize> = Vec::new();
    let execution = execute_prune_with(&branches, PRUNE_BATCH_SIZE, |batch| {
        batches.push(batch.len());
        Ok(())
    });
    assert_eq!(batches, vec![15, 15, 2]);
    assert_eq!(execution.deleted.len(), 32);
    assert!(execution.failed.is_empty());
}

#[test]
fn one_failing_branch_does_not_stop_the_remaining_deletions() {
    let branches: Vec<String> = (0..20).map(|n| format!("work/issue-{n}")).collect();
    let execution = execute_prune_with(&branches, PRUNE_BATCH_SIZE, |batch| {
        if batch.len() == 1 {
            // Per-branch retry after the batch failed.
            if batch[0] == "work/issue-3" {
                return Err("remote rejected work/issue-3".to_string());
            }
            return Ok(());
        }
        if batch.iter().any(|branch| branch == "work/issue-3") {
            return Err("batch push failed".to_string());
        }
        Ok(())
    });
    assert_eq!(execution.deleted.len(), 19);
    assert!(!execution.deleted.iter().any(|b| b == "work/issue-3"));
    assert_eq!(execution.failed.len(), 1);
    assert_eq!(execution.failed[0].branch, "work/issue-3");
    assert!(
        execution.failed[0].reason.contains("remote rejected"),
        "the failure reason must be reported: {}",
        execution.failed[0].reason
    );
}

#[test]
fn dry_run_reports_candidates_and_reasons_without_deleting() {
    let mut remote = FakeRemote {
        merged: BTreeMap::from([("work/issue-1".to_string(), 11)]),
        ..FakeRemote::default()
    };
    let branches = vec!["work/issue-1".to_string(), "work/issue-2".to_string()];
    let report = prune_merged_branches(&mut remote, &branches, true);
    assert!(report.skipped_reason.is_none());
    assert_eq!(report.plan.prune.len(), 1);
    assert_eq!(report.plan.prune[0].branch, "work/issue-1");
    assert_eq!(report.plan.skip[0].branch, "work/issue-2");
    assert_eq!(report.plan.skip[0].reason, PruneSkipReason::NotMerged);
    assert!(report.execution.deleted.is_empty());
    assert!(
        remote.deleted.is_empty(),
        "dry_run must not touch the remote"
    );
}

#[test]
fn apply_deletes_only_the_planned_candidates() {
    let mut remote = FakeRemote {
        merged: BTreeMap::from([
            ("work/issue-1".to_string(), 11),
            ("work/issue-2".to_string(), 12),
        ]),
        open: HashMap::from([("work/issue-2".to_string(), 13)]),
        ..FakeRemote::default()
    };
    let branches = vec!["work/issue-1".to_string(), "work/issue-2".to_string()];
    let report = prune_merged_branches(&mut remote, &branches, false);
    assert_eq!(report.execution.deleted, vec!["work/issue-1".to_string()]);
    assert_eq!(remote.deleted, vec!["work/issue-1".to_string()]);
    assert!(report
        .log_lines()
        .iter()
        .any(|line| line.contains("work/issue-1") && line.contains("deleted")));
}

#[test]
fn a_rate_limited_remote_skips_the_prune_with_a_reason_and_deletes_nothing() {
    for error in [
        "API rate limit exceeded for user",
        "gh: command not found",
        "gh auth status: not logged in",
    ] {
        let mut remote = FakeRemote {
            merged_error: Some(error.to_string()),
            ..FakeRemote::default()
        };
        let branches = vec!["work/issue-1".to_string()];
        let report = prune_merged_branches(&mut remote, &branches, false);
        let reason = report
            .skipped_reason
            .as_deref()
            .expect("an unusable remote must be reported as a skip, not a silent no-op");
        assert!(reason.contains(error), "{reason}");
        assert!(report.plan.prune.is_empty());
        assert!(report.execution.deleted.is_empty());
        assert!(remote.deleted.is_empty());
        assert!(report
            .log_lines()
            .iter()
            .any(|line| line.contains("skipped") && line.contains(error)));
    }
}

#[test]
fn a_branch_already_gone_from_the_remote_is_reported_as_absent_not_failed() {
    let branches = vec!["work/issue-9".to_string()];
    let execution = execute_prune_with(&branches, PRUNE_BATCH_SIZE, |_| {
        Err("error: unable to delete 'work/issue-9': remote ref does not exist".to_string())
    });
    assert!(execution.failed.is_empty(), "{:?}", execution.failed);
    assert!(execution.deleted.is_empty());
    assert_eq!(execution.absent, vec!["work/issue-9".to_string()]);
}

#[test]
fn issue_6394_prune_git_stops_at_maintenance_deadline_without_spending_launch_reserve() {
    use gwt_core::operation_deadline::{now, ScopedOperationClock, ScopedOperationDeadline};
    use gwt_git::merged_branch_prune::{
        count_unmerged_commits, list_remote_work_branches, push_delete_batch, refresh_remote_refs,
    };
    use std::time::{Duration, Instant};

    let repo = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    let git = |root: &std::path::Path, args: &[&str]| {
        let output = gwt_core::process::hidden_command("git")
            .current_dir(root)
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
    };
    git(remote.path(), &["init", "--bare"]);
    git(repo.path(), &["init"]);
    git(
        repo.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(
        repo.path(),
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "--allow-empty",
            "-m",
            "initial",
        ],
    );
    git(
        repo.path(),
        &[
            "push",
            "origin",
            "HEAD:refs/heads/develop",
            "HEAD:refs/heads/work/issue-1",
        ],
    );
    refresh_remote_refs(repo.path()).unwrap();
    assert_eq!(
        count_unmerged_commits(repo.path(), "develop", "work/issue-1"),
        Some(0)
    );

    let started = Instant::now();
    let _clock = ScopedOperationClock::set(started);
    let scan_deadline = started + Duration::from_secs(11);
    let _scan = ScopedOperationDeadline::enter(scan_deadline);
    {
        let maintenance_deadline = scan_deadline - Duration::from_secs(10);
        let _maintenance = ScopedOperationDeadline::enter(maintenance_deadline);
        let _spent = ScopedOperationClock::set(maintenance_deadline);
        assert!(refresh_remote_refs(repo.path())
            .unwrap_err()
            .contains("deadline expired"));
        assert!(list_remote_work_branches(repo.path())
            .unwrap_err()
            .contains("deadline expired"));
        assert_eq!(
            count_unmerged_commits(repo.path(), "develop", "work/issue-1"),
            None
        );
        assert!(
            push_delete_batch(repo.path(), &["work/issue-1".to_string()])
                .unwrap_err()
                .contains("deadline expired")
        );
    }
    let _after_maintenance = ScopedOperationClock::set(scan_deadline - Duration::from_secs(10));
    assert_eq!(scan_deadline.duration_since(now()), Duration::from_secs(10));
    gwt_core::operation_deadline::ensure_remaining("proposal-return").unwrap();
    drop(_scan);
    assert_eq!(
        list_remote_work_branches(repo.path()).unwrap(),
        vec!["work/issue-1"]
    );
}
