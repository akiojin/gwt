//! `branch.*` JSON operations (Issue #3970).
//!
//! `branch.prune_merged` sweeps the `work/issue-*` remote branches that a
//! merged pull request left behind. It shares
//! [`gwt_git::merged_branch_prune::prune_merged_branches`] — and therefore the
//! candidate rules — with the Issue Monitor's post-delivery prune, so the bulk
//! sweep can never delete something the delivery hook would have kept.

use gwt_git::merged_branch_prune::{
    list_remote_work_branches, prune_merged_branches, refresh_remote_refs, GitPruneEnvironment,
    PruneReport,
};
use gwt_github::SpecOpsError;

use super::CliEnv;

/// Command model for the `branch.*` family.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchCommand {
    /// `branch.prune_merged` — delete merged `work/issue-*` remote branches.
    PruneMerged {
        /// Candidates and reasons only; nothing is pushed. Defaults to `true`
        /// so an exploratory call can never delete.
        dry_run: bool,
        /// Base branch the ancestry check runs against.
        base: Option<String>,
        /// Restrict the sweep to these branches; empty means every
        /// `work/issue-*` branch the remote still has.
        branches: Vec<String>,
    },
}

/// Render a [`PruneReport`] as the operation payload.
pub fn report_json(report: &PruneReport, base: &str) -> serde_json::Value {
    serde_json::json!({
        "dry_run": report.dry_run,
        "base": base,
        "skipped_reason": report.skipped_reason,
        "candidates": report
            .plan
            .prune
            .iter()
            .map(|target| serde_json::json!({
                "branch": target.branch,
                "pr_number": target.pr_number,
            }))
            .collect::<Vec<_>>(),
        "kept": report
            .plan
            .skip
            .iter()
            .map(|skipped| serde_json::json!({
                "branch": skipped.branch,
                "reason": skipped.reason.to_string(),
            }))
            .collect::<Vec<_>>(),
        "deleted": report.execution.deleted,
        "absent": report.execution.absent,
        "failed": report
            .execution
            .failed
            .iter()
            .map(|failure| serde_json::json!({
                "branch": failure.branch,
                "reason": failure.reason,
            }))
            .collect::<Vec<_>>(),
        "log_lines": report.log_lines(),
    })
}

pub(super) fn run<E: CliEnv>(
    env: &mut E,
    command: BranchCommand,
    out: &mut String,
) -> Result<i32, SpecOpsError> {
    match command {
        BranchCommand::PruneMerged {
            dry_run,
            base,
            branches,
        } => {
            let base =
                base.unwrap_or_else(|| gwt_git::pr_status::SETTLEMENT_BASE_BRANCH.to_string());
            let repo_path = env.repo_path().to_path_buf();
            let report = run_prune(&repo_path, &base, &branches, dry_run);
            out.push_str(
                &serde_json::to_string_pretty(&report_json(&report, &base))
                    .map_err(super::serde_as_api_error)?,
            );
            out.push('\n');
            Ok(0)
        }
    }
}

/// Refresh, enumerate, then prune. Any remote failure short-circuits into a
/// reported skip rather than an error: an unreadable remote must not look like
/// "nothing to delete" (Issue #3970 AC-5).
fn run_prune(
    repo_path: &std::path::Path,
    base: &str,
    branches: &[String],
    dry_run: bool,
) -> PruneReport {
    let mut report = PruneReport {
        dry_run,
        ..PruneReport::default()
    };
    if let Err(error) = refresh_remote_refs(repo_path) {
        report.skipped_reason = Some(format!("git fetch origin --prune failed: {error}"));
        return report;
    }
    let branches = if branches.is_empty() {
        match list_remote_work_branches(repo_path) {
            Ok(found) => found,
            Err(error) => {
                report.skipped_reason = Some(format!("git ls-remote failed: {error}"));
                return report;
            }
        }
    } else {
        branches.to_vec()
    };
    let mut environment = GitPruneEnvironment::new(repo_path, base);
    prune_merged_branches(&mut environment, &branches, dry_run)
}
