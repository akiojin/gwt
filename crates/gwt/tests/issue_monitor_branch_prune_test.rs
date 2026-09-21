//! Issue #3970 AC-1: the Issue Monitor retires a `work/issue-*` remote branch
//! as soon as it confirms the delivery, and records the outcome in one line.

use std::collections::{BTreeMap, HashMap};

use gwt::issue_monitor_worker::{
    prune_delivered_work_branches_with, IssueMonitorMergeReconciliation,
};
use gwt::MergedIssueDelivery;
use gwt_git::merged_branch_prune::PruneEnvironment;

/// Scripted remote. `delete_error` lets a test drive the failure paths without
/// a real `git push`.
#[derive(Default)]
struct FakeRemote {
    merged: BTreeMap<String, u64>,
    open: HashMap<String, u64>,
    delete_error: Option<String>,
    deleted: Vec<String>,
}

impl PruneEnvironment for FakeRemote {
    fn merged_prs(&mut self) -> Result<BTreeMap<String, u64>, String> {
        Ok(self.merged.clone())
    }

    fn open_prs(&mut self) -> Result<HashMap<String, u64>, String> {
        Ok(self.open.clone())
    }

    fn unmerged_commits(&mut self, _branch: &str) -> Option<u64> {
        Some(0)
    }

    fn delete_batch(&mut self, branches: &[String]) -> Result<(), String> {
        if let Some(error) = &self.delete_error {
            return Err(error.clone());
        }
        self.deleted.extend(branches.iter().cloned());
        Ok(())
    }
}

fn reconciliation(
    merged_issues: &[u64],
    branch: &str,
    pr_number: u64,
) -> IssueMonitorMergeReconciliation {
    IssueMonitorMergeReconciliation {
        merged: merged_issues.to_vec(),
        deliveries: BTreeMap::from([(
            branch.to_string(),
            MergedIssueDelivery {
                pr_number,
                merge_sha: Some("c0ffee".to_string()),
                merged_at: Some("2026-09-06T00:00:00Z".to_string()),
            },
        )]),
    }
}

#[test]
fn a_confirmed_delivery_deletes_its_work_branch_and_logs_one_line() {
    let mut remote = FakeRemote {
        merged: BTreeMap::from([("work/issue-3970".to_string(), 4001)]),
        ..FakeRemote::default()
    };
    let report = prune_delivered_work_branches_with(
        &mut remote,
        &reconciliation(&[3970], "work/issue-3970", 4001),
        &["work/issue-3970".to_string()],
    );
    assert_eq!(remote.deleted, vec!["work/issue-3970".to_string()]);
    assert_eq!(
        report.execution.deleted,
        vec!["work/issue-3970".to_string()]
    );
    let lines = report.log_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("work/issue-3970"), "{}", lines[0]);
    assert!(lines[0].contains("deleted"), "{}", lines[0]);
    assert!(lines[0].contains("4001"), "{}", lines[0]);
}

#[test]
fn a_scan_that_confirmed_no_delivery_touches_no_branch() {
    let mut remote = FakeRemote {
        merged: BTreeMap::from([("work/issue-3970".to_string(), 4001)]),
        ..FakeRemote::default()
    };
    let report = prune_delivered_work_branches_with(
        &mut remote,
        &reconciliation(&[], "work/issue-3970", 4001),
        &["work/issue-3970".to_string()],
    );
    assert!(remote.deleted.is_empty());
    assert!(report.log_lines().is_empty());
}

#[test]
fn a_branch_the_remote_already_dropped_is_logged_as_absent() {
    let mut remote = FakeRemote {
        merged: BTreeMap::from([("work/issue-3970".to_string(), 4001)]),
        delete_error: Some(
            "error: unable to delete 'work/issue-3970': remote ref does not exist".to_string(),
        ),
        ..FakeRemote::default()
    };
    let report = prune_delivered_work_branches_with(
        &mut remote,
        &reconciliation(&[3970], "work/issue-3970", 4001),
        &["work/issue-3970".to_string()],
    );
    assert!(report.execution.failed.is_empty());
    let lines = report.log_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("already absent"), "{}", lines[0]);
}

#[test]
fn a_rejected_deletion_is_logged_with_its_reason() {
    let mut remote = FakeRemote {
        merged: BTreeMap::from([("work/issue-3970".to_string(), 4001)]),
        delete_error: Some("remote: protected branch hook declined".to_string()),
        ..FakeRemote::default()
    };
    let report = prune_delivered_work_branches_with(
        &mut remote,
        &reconciliation(&[3970], "work/issue-3970", 4001),
        &["work/issue-3970".to_string()],
    );
    assert_eq!(report.execution.failed.len(), 1);
    let lines = report.log_lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("failed"), "{}", lines[0]);
    assert!(lines[0].contains("hook declined"), "{}", lines[0]);
}

#[test]
fn the_delivery_hook_and_the_bulk_sweep_agree_on_every_candidate() {
    // Issue #3970 AC-3: `branch.prune_merged` must not be able to delete a
    // branch the delivery hook would keep. Both entrances run the same facts
    // through the same planner, so the plans are identical by construction —
    // this pins that, and fails the moment one grows its own rules.
    let branches = vec![
        "work/issue-1".to_string(),
        "work/issue-2".to_string(),
        "work/issue-3".to_string(),
        "release/9.9.0".to_string(),
    ];
    let merged = BTreeMap::from([
        ("work/issue-1".to_string(), 11),
        ("work/issue-2".to_string(), 12),
        ("release/9.9.0".to_string(), 13),
    ]);
    let open = HashMap::from([("work/issue-2".to_string(), 21)]);

    let mut hook_remote = FakeRemote {
        merged: merged.clone(),
        open: open.clone(),
        ..FakeRemote::default()
    };
    let hook = prune_delivered_work_branches_with(
        &mut hook_remote,
        &reconciliation(&[1], "work/issue-1", 11),
        &branches,
    );

    let mut bulk_remote = FakeRemote {
        merged,
        open,
        ..FakeRemote::default()
    };
    let bulk =
        gwt_git::merged_branch_prune::prune_merged_branches(&mut bulk_remote, &branches, false);

    assert_eq!(hook.plan, bulk.plan);
    assert_eq!(hook_remote.deleted, bulk_remote.deleted);
    assert_eq!(hook_remote.deleted, vec!["work/issue-1".to_string()]);
}

#[test]
fn a_delivery_sweep_keeps_branches_the_shared_rules_refuse() {
    // The sweep runs over every `work/issue-*` branch the remote still has, so
    // the same pass that retires the delivered branch must leave an unrelated
    // branch with an open PR alone.
    let mut remote = FakeRemote {
        merged: BTreeMap::from([
            ("work/issue-3970".to_string(), 4001),
            ("work/issue-1234".to_string(), 4002),
        ]),
        open: HashMap::from([("work/issue-1234".to_string(), 4100)]),
        ..FakeRemote::default()
    };
    let report = prune_delivered_work_branches_with(
        &mut remote,
        &reconciliation(&[3970], "work/issue-3970", 4001),
        &[
            "work/issue-3970".to_string(),
            "work/issue-1234".to_string(),
            "develop".to_string(),
        ],
    );
    assert_eq!(remote.deleted, vec!["work/issue-3970".to_string()]);
    assert_eq!(report.plan.skip.len(), 2);
    assert!(report
        .log_lines()
        .iter()
        .any(|line| line.contains("work/issue-1234") && line.contains("open PR #4100")));
    assert!(report
        .log_lines()
        .iter()
        .any(|line| line.contains("develop") && line.contains("protected")));
}
