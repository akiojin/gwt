//! Prune `work/issue-*` remote branches whose pull request already merged
//! (Issue #3970).
//!
//! gwt creates one `work/issue-N` branch per launch and never deletes it, so
//! every merged PR leaves a branch behind on the remote. 265 of them had piled
//! up by 2026-09-05, and the Issue Monitor's open-PR readback walks that list
//! once per scan — the accumulation is what pushed a scan past its budget and
//! stopped launches (Issue #3963).
//!
//! Two entrances share one decision. The Issue Monitor deletes a head branch
//! right after it confirms the delivery; `branch.prune_merged` sweeps whatever
//! already accumulated. Both call [`prune_merged_branches`], so a branch the
//! sweep would refuse is a branch the delivery hook refuses too.
//!
//! Every rule here is fail-closed: a branch is deleted only when the evidence
//! positively says it is safe, never because the evidence could not be read.

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

/// Remote refspecs pushed per `git push` invocation. Matches the batch size the
/// 2026-09-05 manual cleanup used; large enough that 265 branches cost ~18
/// pushes, small enough that one bad ref only forces a 15-way retry.
pub const PRUNE_BATCH_SIZE: usize = 15;

/// Branch name prefixes that are protected in addition to
/// [`crate::is_protected_branch`]'s exact names.
const PROTECTED_PREFIXES: &[&str] = &["release/"];

/// Branch prefix the bulk sweep enumerates from the remote.
pub const WORK_BRANCH_GLOB: &str = "work/issue-*";

/// Everything the prune decision needs to know about one remote branch.
///
/// `unmerged_commits` is `None` when the count could not be established (a
/// missing tracking ref, an unreachable object). That is not zero — it is
/// "unknown", and unknown never deletes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneCandidateFacts {
    pub branch: String,
    /// Number of the merged PR whose head this branch was, if any.
    pub merged_pr: Option<u64>,
    /// Number of an open PR still using this branch as its head, if any.
    pub open_pr: Option<u64>,
    /// Commits on the branch that are not reachable from the base branch.
    pub unmerged_commits: Option<u64>,
}

/// Why one branch was left on the remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneSkipReason {
    /// A base or release branch. Refused before any other evidence is read.
    Protected,
    /// No merged pull request claims this branch as its head.
    NotMerged,
    /// An open pull request still uses the branch.
    OpenPr(u64),
    /// The branch carries commits the base branch does not have.
    UnmergedCommits(u64),
    /// The unmerged-commit count could not be established.
    AheadUnknown,
}

impl std::fmt::Display for PruneSkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Protected => write!(f, "protected branch"),
            Self::NotMerged => write!(f, "no merged PR"),
            Self::OpenPr(number) => write!(f, "open PR #{number}"),
            Self::UnmergedCommits(count) => write!(f, "{count} unmerged commit(s)"),
            Self::AheadUnknown => write!(f, "unmerged commit count unknown"),
        }
    }
}

/// The verdict for one branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PruneDecision {
    Prune { pr_number: u64 },
    Skip(PruneSkipReason),
}

/// One branch cleared for deletion, with the delivery that cleared it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneTarget {
    pub branch: String,
    pub pr_number: u64,
}

/// One branch left alone, with the reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneSkipped {
    pub branch: String,
    pub reason: PruneSkipReason,
}

/// What a prune pass decided, before anything was pushed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    pub prune: Vec<PruneTarget>,
    pub skip: Vec<PruneSkipped>,
}

/// One branch the remote refused to delete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PruneFailure {
    pub branch: String,
    pub reason: String,
}

/// What the pushes actually did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneExecution {
    pub deleted: Vec<String>,
    /// Branches the remote no longer had. Not a failure — the goal is already
    /// met — but reported separately so a caller can tell the two apart.
    pub absent: Vec<String>,
    pub failed: Vec<PruneFailure>,
}

/// The full result of one prune pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PruneReport {
    pub plan: PrunePlan,
    pub execution: PruneExecution,
    /// Set when the pass could not run at all — `gh` missing, unauthenticated,
    /// or inside a rate-limit window. The caller must treat this as "try
    /// later", never as "nothing to delete".
    pub skipped_reason: Option<String>,
    /// True when the caller asked for candidates only.
    pub dry_run: bool,
}

impl PruneReport {
    /// One line per branch, suitable for `gwt.log` or a Board post
    /// (Issue #3970 AC-1).
    pub fn log_lines(&self) -> Vec<String> {
        if let Some(reason) = &self.skipped_reason {
            return vec![format!("merged-branch-prune: skipped ({reason})")];
        }
        let mut lines = Vec::new();
        for branch in &self.execution.deleted {
            let pr = self
                .plan
                .prune
                .iter()
                .find(|target| &target.branch == branch)
                .map(|target| target.pr_number);
            match pr {
                Some(number) => lines.push(format!(
                    "merged-branch-prune: {branch} deleted (PR #{number})"
                )),
                None => lines.push(format!("merged-branch-prune: {branch} deleted")),
            }
        }
        for branch in &self.execution.absent {
            lines.push(format!("merged-branch-prune: {branch} already absent"));
        }
        for failure in &self.execution.failed {
            lines.push(format!(
                "merged-branch-prune: {} failed ({})",
                failure.branch, failure.reason
            ));
        }
        for skipped in &self.plan.skip {
            lines.push(format!(
                "merged-branch-prune: {} kept ({})",
                skipped.branch, skipped.reason
            ));
        }
        lines
    }
}

/// True when the branch must never be deleted regardless of PR state.
pub fn is_prune_protected(branch: &str) -> bool {
    let bare = branch.strip_prefix("origin/").unwrap_or(branch);
    crate::is_protected_branch(bare)
        || PROTECTED_PREFIXES
            .iter()
            .any(|prefix| bare.starts_with(prefix))
}

/// Decide one branch (Issue #3970 AC-2).
///
/// Protection is checked first so a protected branch can never be deleted by a
/// mis-shaped fact set, and the two positive conditions — a merged PR and zero
/// unmerged commits — must both hold.
pub fn classify_prune_candidate(facts: &PruneCandidateFacts) -> PruneDecision {
    if is_prune_protected(&facts.branch) {
        return PruneDecision::Skip(PruneSkipReason::Protected);
    }
    if let Some(number) = facts.open_pr {
        return PruneDecision::Skip(PruneSkipReason::OpenPr(number));
    }
    let Some(pr_number) = facts.merged_pr else {
        return PruneDecision::Skip(PruneSkipReason::NotMerged);
    };
    match facts.unmerged_commits {
        None => PruneDecision::Skip(PruneSkipReason::AheadUnknown),
        Some(0) => PruneDecision::Prune { pr_number },
        Some(count) => PruneDecision::Skip(PruneSkipReason::UnmergedCommits(count)),
    }
}

/// Apply [`classify_prune_candidate`] across a candidate set, preserving order.
pub fn plan_prune(facts: &[PruneCandidateFacts]) -> PrunePlan {
    let mut plan = PrunePlan::default();
    for candidate in facts {
        match classify_prune_candidate(candidate) {
            PruneDecision::Prune { pr_number } => plan.prune.push(PruneTarget {
                branch: candidate.branch.clone(),
                pr_number,
            }),
            PruneDecision::Skip(reason) => plan.skip.push(PruneSkipped {
                branch: candidate.branch.clone(),
                reason,
            }),
        }
    }
    plan
}

/// True when a push error means the ref was already gone.
fn is_missing_remote_ref(error: &str) -> bool {
    let lowered = error.to_ascii_lowercase();
    lowered.contains("remote ref does not exist") || lowered.contains("unable to delete")
}

/// Delete `branches` in batched pushes, degrading to one push per branch when a
/// batch fails (Issue #3970 AC-4).
///
/// A batched refspec push is all-or-nothing per invocation, so a single bad ref
/// would otherwise take its whole batch down with it. Retrying the failed batch
/// one branch at a time is what turns that into an isolated failure while the
/// other 14 still get deleted, and it is also the only way to attribute the
/// error to a branch.
pub fn execute_prune_with<F>(
    branches: &[String],
    batch_size: usize,
    mut push_delete: F,
) -> PruneExecution
where
    F: FnMut(&[String]) -> Result<(), String>,
{
    let mut execution = PruneExecution::default();
    let batch_size = batch_size.max(1);
    for batch in branches.chunks(batch_size) {
        match push_delete(batch) {
            Ok(()) => execution.deleted.extend(batch.iter().cloned()),
            Err(batch_error) if batch.len() == 1 => {
                record_single_failure(&mut execution, &batch[0], batch_error)
            }
            Err(_) => {
                for branch in batch {
                    let single = std::slice::from_ref(branch);
                    match push_delete(single) {
                        Ok(()) => execution.deleted.push(branch.clone()),
                        Err(error) => record_single_failure(&mut execution, branch, error),
                    }
                }
            }
        }
    }
    execution
}

fn record_single_failure(execution: &mut PruneExecution, branch: &str, error: String) {
    if is_missing_remote_ref(&error) {
        execution.absent.push(branch.to_string());
    } else {
        execution.failed.push(PruneFailure {
            branch: branch.to_string(),
            reason: error,
        });
    }
}

/// The remote-facing operations one prune pass needs. Production wires this to
/// `gh` and `git`; tests script it.
pub trait PruneEnvironment {
    /// `head branch -> merged PR number` for every merged PR.
    fn merged_prs(&mut self) -> Result<BTreeMap<String, u64>, String>;
    /// `head branch -> open PR number` for every open PR.
    fn open_prs(&mut self) -> Result<HashMap<String, u64>, String>;
    /// Commits on `branch` that the base branch does not have; `None` when the
    /// count cannot be established.
    fn unmerged_commits(&mut self, branch: &str) -> Option<u64>;
    /// Delete one batch of branches from the remote.
    fn delete_batch(&mut self, branches: &[String]) -> Result<(), String>;
}

/// Run one prune pass over `branches`.
///
/// When the PR inventory cannot be read the pass reports a skip and deletes
/// nothing (Issue #3970 AC-5); the caller — the Issue Monitor scan in
/// particular — keeps going.
pub fn prune_merged_branches<E: PruneEnvironment>(
    env: &mut E,
    branches: &[String],
    dry_run: bool,
) -> PruneReport {
    let mut report = PruneReport {
        dry_run,
        ..PruneReport::default()
    };
    if branches.is_empty() {
        return report;
    }
    let merged = match env.merged_prs() {
        Ok(merged) => merged,
        Err(error) => {
            report.skipped_reason = Some(error);
            return report;
        }
    };
    let open = match env.open_prs() {
        Ok(open) => open,
        Err(error) => {
            report.skipped_reason = Some(error);
            return report;
        }
    };
    let facts = branches
        .iter()
        .map(|branch| {
            let merged_pr = merged.get(branch).copied();
            let open_pr = open.get(branch).copied();
            // The ancestry probe costs a git call per branch, so it is only
            // worth paying once the cheap evidence already points at deletion.
            let unmerged_commits =
                if merged_pr.is_some() && open_pr.is_none() && !is_prune_protected(branch) {
                    env.unmerged_commits(branch)
                } else {
                    None
                };
            PruneCandidateFacts {
                branch: branch.clone(),
                merged_pr,
                open_pr,
                unmerged_commits,
            }
        })
        .collect::<Vec<_>>();
    report.plan = plan_prune(&facts);
    if dry_run || report.plan.prune.is_empty() {
        return report;
    }
    let targets = report
        .plan
        .prune
        .iter()
        .map(|target| target.branch.clone())
        .collect::<Vec<_>>();
    report.execution =
        execute_prune_with(&targets, PRUNE_BATCH_SIZE, |batch| env.delete_batch(batch));
    report
}

/// Production [`PruneEnvironment`]: `gh` for the PR inventory, `git` for
/// ancestry and deletion.
pub struct GitPruneEnvironment {
    repo_path: PathBuf,
    base_branch: String,
    merged_prs: Option<BTreeMap<String, u64>>,
}

impl GitPruneEnvironment {
    pub fn new(repo_path: &Path, base_branch: &str) -> Self {
        Self {
            repo_path: repo_path.to_path_buf(),
            base_branch: base_branch.to_string(),
            merged_prs: None,
        }
    }

    /// Reuse a merged-PR inventory the caller already fetched. The Issue
    /// Monitor scan has one in hand, and re-querying it would double the
    /// prune's `gh` cost inside the budget Issue #3963 was about.
    #[must_use]
    pub fn with_merged_prs(mut self, merged: BTreeMap<String, u64>) -> Self {
        self.merged_prs = Some(merged);
        self
    }
}

impl PruneEnvironment for GitPruneEnvironment {
    fn merged_prs(&mut self) -> Result<BTreeMap<String, u64>, String> {
        if let Some(merged) = &self.merged_prs {
            return Ok(merged.clone());
        }
        let deliveries = crate::pr_status::fetch_merged_pr_deliveries(&self.repo_path)
            .map_err(|error| error.to_string())?;
        Ok(deliveries
            .deliveries
            .into_iter()
            .map(|(branch, delivery)| (branch, delivery.number))
            .collect())
    }

    fn open_prs(&mut self) -> Result<HashMap<String, u64>, String> {
        crate::pr_status::try_fetch_open_pr_numbers_by_branch(&self.repo_path)
            .map_err(|error| error.to_string())
    }

    fn unmerged_commits(&mut self, branch: &str) -> Option<u64> {
        count_unmerged_commits(&self.repo_path, &self.base_branch, branch)
    }

    fn delete_batch(&mut self, branches: &[String]) -> Result<(), String> {
        push_delete_batch(&self.repo_path, branches)
    }
}

/// `git rev-list --count origin/<base>..origin/<branch>`.
///
/// Any failure — a missing tracking ref, an unreadable object — answers `None`
/// so the branch is kept rather than deleted on an unverified guess.
pub fn count_unmerged_commits(repo_path: &Path, base_branch: &str, branch: &str) -> Option<u64> {
    let range = format!("origin/{base_branch}..origin/{branch}");
    let output = gwt_core::process::run_git_logged(
        &["rev-list", "--count", range.as_str()],
        Some(repo_path),
    )
    .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

/// Refresh remote-tracking refs so ancestry counts reflect the remote.
///
/// `--prune` also drops tracking refs for branches a previous pass already
/// deleted, which is what keeps a repeated sweep from re-proposing them.
pub fn refresh_remote_refs(repo_path: &Path) -> Result<(), String> {
    let output = gwt_core::process::run_git_logged(
        &["fetch", "origin", "--prune", "--quiet"],
        Some(repo_path),
    )
    .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

/// The delete refspecs one batched `git push` sends.
pub fn delete_refspecs(branches: &[String]) -> Vec<String> {
    branches
        .iter()
        .map(|branch| format!(":refs/heads/{branch}"))
        .collect()
}

/// `git push origin :refs/heads/<a> :refs/heads/<b> ...` for one batch.
pub fn push_delete_batch(repo_path: &Path, branches: &[String]) -> Result<(), String> {
    if branches.is_empty() {
        return Ok(());
    }
    for branch in branches {
        if is_prune_protected(branch) {
            return Err(format!("refusing to delete protected branch: {branch}"));
        }
    }
    let refspecs = delete_refspecs(branches);
    let mut args = vec!["push", "origin"];
    args.extend(refspecs.iter().map(String::as_str));
    let output = gwt_core::process::run_git_logged(&args, Some(repo_path))
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

/// Enumerate `work/issue-*` branches that exist on the remote.
pub fn list_remote_work_branches(repo_path: &Path) -> Result<Vec<String>, String> {
    let output = gwt_core::process::run_git_logged(
        &["ls-remote", "--heads", "origin", WORK_BRANCH_GLOB],
        Some(repo_path),
    )
    .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(parse_ls_remote_heads(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

/// Parse `git ls-remote --heads` output into bare branch names.
pub fn parse_ls_remote_heads(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter_map(|reference| reference.strip_prefix("refs/heads/"))
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_remote_heads_parse_drops_the_ref_prefix() {
        let stdout = "abc123\trefs/heads/work/issue-1\ndef456\trefs/heads/work/issue-2\n";
        assert_eq!(
            parse_ls_remote_heads(stdout),
            vec!["work/issue-1".to_string(), "work/issue-2".to_string()]
        );
    }

    #[test]
    fn a_batch_becomes_one_delete_refspec_per_branch() {
        assert_eq!(
            delete_refspecs(&["work/issue-1".to_string(), "work/issue-2".to_string()]),
            vec![
                ":refs/heads/work/issue-1".to_string(),
                ":refs/heads/work/issue-2".to_string()
            ]
        );
    }

    #[test]
    fn push_delete_batch_refuses_a_protected_branch_before_spawning_git() {
        let error = push_delete_batch(Path::new("/nonexistent"), &["develop".to_string()])
            .expect_err("protected branches must be refused");
        assert!(error.contains("develop"), "{error}");
    }

    #[test]
    fn missing_remote_ref_errors_are_recognized() {
        assert!(is_missing_remote_ref(
            "error: unable to delete 'work/issue-1': remote ref does not exist"
        ));
        assert!(!is_missing_remote_ref("remote rejected the push"));
    }
}
