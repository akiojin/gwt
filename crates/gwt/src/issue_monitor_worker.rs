use std::{fmt, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    IssueMonitorCandidateSource, IssueMonitorInboxItem, IssueMonitorIssue, IssueMonitorIssueState,
    IssueMonitorScanSummary, IssueMonitorState,
};
use gwt_github::{Cache, IssueState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueMonitorDaemonPayload {
    pub event: String,
    pub payload: Value,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedIssueMonitorCandidates {
    pub issues: Vec<IssueMonitorIssue>,
    pub source: IssueMonitorCandidateSource,
    /// The live-list failure that forced a cache fallback. Kept alongside the
    /// stale read model so timeout/error status is never rendered as healthy.
    pub live_error: Option<String>,
}

impl LoadedIssueMonitorCandidates {
    pub fn authorizes_remote_effects(&self) -> bool {
        self.source == IssueMonitorCandidateSource::Live && self.live_error.is_none()
    }
}

/// External stage owned by one side-effect-free Issue Monitor scan proposal.
/// The stage is retained in errors so operators and tests can distinguish the
/// boundary that failed instead of observing a lossy bool/None/default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueMonitorScanStage {
    RemoteResolution,
    CandidateLoad,
    MergeReconciliation,
    DefaultBaseBranch,
    BranchProtection,
    OpenPrReadback,
    HeadShaReadback,
    PrDiffReadback,
    StatusCheckReadback,
    MergeCommitReadback,
    ClaimCompletionReadback,
    ProposalReturn,
}

impl IssueMonitorScanStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RemoteResolution => "remote-resolution",
            Self::CandidateLoad => "candidate-load",
            Self::MergeReconciliation => "merge-reconciliation",
            Self::DefaultBaseBranch => "default-base-branch",
            Self::BranchProtection => "branch-protection",
            Self::OpenPrReadback => "open-pr-readback",
            Self::HeadShaReadback => "head-sha-readback",
            Self::PrDiffReadback => "pr-diff-readback",
            Self::StatusCheckReadback => "status-check-readback",
            Self::MergeCommitReadback => "merge-commit-readback",
            Self::ClaimCompletionReadback => "claim-completion-readback",
            Self::ProposalReturn => "proposal-return",
        }
    }
}

impl fmt::Display for IssueMonitorScanStage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueMonitorScanFailure {
    pub stage: IssueMonitorScanStage,
    pub detail: String,
}

impl IssueMonitorScanFailure {
    pub fn new(stage: IssueMonitorScanStage, detail: impl Into<String>) -> Self {
        Self {
            stage,
            detail: detail.into(),
        }
    }
}

impl fmt::Display for IssueMonitorScanFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "issue monitor scan failed at {} stage: {}",
            self.stage, self.detail
        )
    }
}

impl std::error::Error for IssueMonitorScanFailure {}

pub fn ensure_scan_deadline(stage: IssueMonitorScanStage) -> Result<(), IssueMonitorScanFailure> {
    gwt_core::operation_deadline::ensure_remaining(stage.as_str())
        .map(|_| ())
        .map_err(|error| IssueMonitorScanFailure::new(stage, error.to_string()))
}

pub(crate) fn run_scan_stage<T, E>(
    stage: IssueMonitorScanStage,
    operation: impl FnOnce() -> Result<T, E>,
) -> Result<T, IssueMonitorScanFailure>
where
    E: fmt::Display,
{
    ensure_scan_deadline(stage)?;
    let value =
        operation().map_err(|error| IssueMonitorScanFailure::new(stage, error.to_string()))?;
    ensure_scan_deadline(stage)?;
    Ok(value)
}

pub fn issue_monitor_daemon_payloads(
    monitor: &mut IssueMonitorState,
    gui_connected: bool,
) -> Vec<IssueMonitorDaemonPayload> {
    monitor.set_gui_connected(gui_connected);
    let mut payloads = Vec::new();
    if gui_connected {
        for request in monitor.take_pending_launch_requests() {
            let delivery_id = request.delivery_id.clone();
            payloads.push(IssueMonitorDaemonPayload {
                event: "launch_request".to_string(),
                payload: serde_json::json!({
                    "issue_number": request.issue_number,
                    "branch_name": request.branch_name,
                    "linked_issue_kind": request.linked_issue_kind,
                    "delivery_id": delivery_id,
                }),
            });
            if request.delivery_id.is_none() {
                payloads.push(IssueMonitorDaemonPayload {
                    event: "toast".to_string(),
                    payload: serde_json::json!({
                        "level": "info",
                        "message": "Issue Monitor launch requested",
                        "issue_number": request.issue_number,
                    }),
                });
            }
        }
        // SPEC #3200 Option A: surface review-agent spawn requests to the GUI.
        for dispatch in monitor.take_pending_review_dispatches() {
            payloads.push(IssueMonitorDaemonPayload {
                event: "review_dispatch".to_string(),
                payload: serde_json::to_value(&dispatch).expect("review dispatch serializes"),
            });
        }
        // SPEC #3200 FR-034 (T-111): surface unattended autonomous lifecycle
        // transitions (merged / needs-human / retry / auto-merge armed) as
        // toasts. Drained only while a GUI is connected so notices queued during
        // a fully-unattended window still reach the operator on the next connect.
        for notice in monitor.take_autonomous_notices() {
            payloads.push(IssueMonitorDaemonPayload {
                event: "toast".to_string(),
                payload: serde_json::json!({
                    "level": notice.level,
                    "message": notice.message,
                    "issue_number": notice.issue_number,
                }),
            });
        }
    }

    payloads.extend(issue_monitor_read_only_daemon_payloads(monitor));
    payloads
}

/// Build the recovery-safe projection without consuming any delivery-bearing
/// outbox. Recovery-blocked daemons use this path because they may expose
/// status and inbox state, but cannot authorize launch/review/toast delivery.
pub fn issue_monitor_read_only_daemon_payloads(
    monitor: &IssueMonitorState,
) -> Vec<IssueMonitorDaemonPayload> {
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    vec![
        IssueMonitorDaemonPayload {
            event: "status".to_string(),
            payload: serde_json::to_value(monitor.status_view_at(&now))
                .expect("issue monitor status serializes"),
        },
        IssueMonitorDaemonPayload {
            event: "inbox".to_string(),
            payload: serde_json::to_value(monitor.inbox.clone())
                .expect("issue monitor inbox serializes"),
        },
    ]
}

pub fn load_open_issue_monitor_candidates(
    owner: &str,
    repo: &str,
) -> Result<Vec<IssueMonitorIssue>, String> {
    let issues = gwt_git::issue::fetch_issues(owner, repo).map_err(|error| error.to_string())?;
    Ok(issues
        .into_iter()
        .map(|issue| IssueMonitorIssue {
            number: issue.number,
            title: issue.title,
            labels: issue.labels,
            state: if issue.state.eq_ignore_ascii_case("closed") {
                IssueMonitorIssueState::Closed
            } else {
                IssueMonitorIssueState::Open
            },
            body: issue.body,
            url: (!issue.url.is_empty()).then_some(issue.url),
        })
        .collect())
}

pub fn load_open_issue_monitor_candidates_for_repo_path(
    repo_path: &Path,
    owner: &str,
    repo: &str,
) -> Result<Vec<IssueMonitorIssue>, String> {
    load_open_issue_monitor_candidates_for_repo_path_with_provenance(repo_path, owner, repo)
        .map(|loaded| loaded.issues)
}

/// Load live candidates when available, retaining typed provenance for capped
/// (therefore incomplete) live lists and cache fallbacks. The existing
/// Vec-returning API above remains a compatibility wrapper.
pub fn load_open_issue_monitor_candidates_for_repo_path_with_provenance(
    repo_path: &Path,
    owner: &str,
    repo: &str,
) -> Result<LoadedIssueMonitorCandidates, String> {
    let live_error = match load_open_issue_monitor_candidates(owner, repo) {
        Ok(issues) => {
            let source = live_candidate_source(issues.len());
            return Ok(LoadedIssueMonitorCandidates {
                issues,
                source,
                live_error: None,
            });
        }
        Err(error) => error,
    };
    let cache_roots = [
        crate::issue_cache::issue_cache_root_for_repo_path(repo_path),
        Some(crate::issue_cache::issue_cache_root_for_repo_slug(
            owner, repo,
        )),
    ];
    let cache_results = cache_roots.into_iter().flatten().map(|cache_root| {
        let result = load_cached_issue_monitor_candidates(&cache_root);
        if let Err(error) = &result {
            tracing::warn!(
                "issue monitor cache fallback failed for {}: {error}",
                cache_root.display()
            );
        }
        result
    });
    resolve_loaded_issue_monitor_candidates(Err(live_error), cache_results)
}

fn resolve_loaded_issue_monitor_candidates<I>(
    live_result: Result<Vec<IssueMonitorIssue>, String>,
    cache_results: I,
) -> Result<LoadedIssueMonitorCandidates, String>
where
    I: IntoIterator<Item = Result<Vec<IssueMonitorIssue>, String>>,
{
    match live_result {
        Ok(issues) => {
            let source = live_candidate_source(issues.len());
            Ok(LoadedIssueMonitorCandidates {
                issues,
                source,
                live_error: None,
            })
        }
        Err(live_error) => {
            for issues in cache_results.into_iter().flatten() {
                if !issues.is_empty() {
                    return Ok(LoadedIssueMonitorCandidates {
                        issues,
                        source: IssueMonitorCandidateSource::Cache,
                        live_error: Some(live_error),
                    });
                }
            }
            Err(live_error)
        }
    }
}

fn live_candidate_source(issue_count: usize) -> IssueMonitorCandidateSource {
    let configured_limit = gwt_git::issue::GITHUB_ISSUE_LIST_LIMIT
        .parse::<usize>()
        .unwrap_or(usize::MAX);
    if issue_count < configured_limit {
        IssueMonitorCandidateSource::Live
    } else {
        IssueMonitorCandidateSource::LiveIncomplete
    }
}

/// Shared loader-to-state transition. Cache snapshots still follow the normal
/// candidate scan, but only Live provenance can unlock the one-shot historical
/// failure migration in the canonical core transition.
pub fn scan_loaded_issue_monitor_candidates(
    monitor: &mut IssueMonitorState,
    loaded: &LoadedIssueMonitorCandidates,
    repo_path: &Path,
    now: &str,
) -> IssueMonitorScanSummary {
    scan_loaded_issue_monitor_candidates_for_project_tab(monitor, loaded, repo_path, None, now)
}

/// GUI-bound variant of the shared loader transition. The project tab id is
/// optional because daemon scans have repository provenance but no GUI tab
/// identity; they still reconcile repo-absent in-flight rows on Live input.
pub fn scan_loaded_issue_monitor_candidates_for_project_tab(
    monitor: &mut IssueMonitorState,
    loaded: &LoadedIssueMonitorCandidates,
    repo_path: &Path,
    expected_project_tab_id: Option<&str>,
    now: &str,
) -> IssueMonitorScanSummary {
    let summary =
        crate::issue_monitor::scan_issue_monitor_candidates_for_project_tab_with_provenance(
            monitor,
            &loaded.issues,
            loaded.source,
            repo_path,
            expected_project_tab_id,
            now,
        );
    if let Some(error) = &loaded.live_error {
        monitor.record_scan_error(
            now,
            format!("issue list failed; using cache fallback: {error}"),
        );
    }
    summary
}

/// Issue #3225: GitHub-derived completion probe for the claim loop — "does
/// this issue have a linked PR that is already MERGED?". Uses the issue's
/// timeline (cross-referenced / connected PRs), so it catches fixes merged via
/// ANY branch, not just the monitor's own `work/issue-N`. Fails open (false)
/// on errors so a transient gh failure never blocks real work.
pub fn issue_completed_by_merged_pr(owner: &str, repo: &str, issue_number: u64) -> bool {
    match try_issue_completed_by_merged_pr(owner, repo, issue_number) {
        Ok(completed) => completed,
        Err(error) => {
            tracing::debug!(
                issue = issue_number,
                error = %error,
                "issue monitor completion probe failed (fail-open)"
            );
            false
        }
    }
}

/// Checked completion probe used by scan proposal transactions.
pub fn try_issue_completed_by_merged_pr(
    owner: &str,
    repo: &str,
    issue_number: u64,
) -> Result<bool, IssueMonitorScanFailure> {
    let prs = run_scan_stage(IssueMonitorScanStage::ClaimCompletionReadback, || {
        crate::cli::issue::fetch_linked_prs_via_gh(
            owner,
            repo,
            gwt_github::IssueNumber(issue_number),
        )
    })?;
    // codex #3226 review: only a PR that actually CLOSES the issue counts — a
    // merged PR that merely references it (Refs #N / partial work) is not done.
    Ok(prs
        .iter()
        .any(|pr| pr.will_close_target && pr.state.eq_ignore_ascii_case("merged")))
}

/// Mark any active launched Issue whose work branch has a merged PR as
/// `Merged`, freeing the active slot. Skips the network call when nothing is
/// launched, and leaves work launched when the PR query fails (so a transient
/// error never closes the slot on a false signal). Query failures are returned
/// so the scan owner can surface them after its final state rebase.
pub fn reconcile_issue_monitor_merges(
    monitor: &mut IssueMonitorState,
    repo_path: &Path,
) -> gwt_core::Result<Vec<u64>> {
    if monitor.active_launched_branches().is_empty() {
        return Ok(Vec::new());
    }
    let merged_branches = gwt_git::pr_status::fetch_merged_pr_branches(repo_path)?;
    let merged = monitor.reconcile_merged_branches(&merged_branches);
    if !merged.is_empty() {
        tracing::info!(
            issues = ?merged,
            "issue monitor marked merged work and freed active slots"
        );
    }
    Ok(merged)
}

/// Parse `git symbolic-ref --short refs/remotes/origin/HEAD` output (e.g.
/// `origin/main`) into the bare default branch name. Fail-closed to `main`.
pub fn parse_default_base_branch(symbolic_ref_stdout: &str) -> String {
    let trimmed = symbolic_ref_stdout.trim();
    let name = trimmed.strip_prefix("origin/").unwrap_or(trimmed);
    if name.is_empty() {
        "main".to_string()
    } else {
        name.to_string()
    }
}

/// Branch assumed when `origin/HEAD` cannot be read.
const DEFAULT_BASE_BRANCH_FALLBACK: &str = "main";

/// Resolve the repo's default base branch (the branch autonomous PRs merge
/// into) via `origin/HEAD`. Fail-closed to `main` on any failure.
pub fn resolve_default_base_branch(repo_path: &Path) -> String {
    try_resolve_default_base_branch(repo_path).unwrap_or_else(|error| {
        tracing::warn!(error = %error, "issue monitor default base branch unavailable");
        DEFAULT_BASE_BRANCH_FALLBACK.to_string()
    })
}

/// Checked default-base resolution used by deadline-integral scans.
///
/// The scan deadline is the only condition that may abort the caller here.
/// `origin/HEAD` is an optional local hint — a repository that never configured
/// it (a bare `git init` under a workspace home, for example) is a normal
/// state, not a scan failure. Treating it as one disabled the whole autonomous
/// progress loop before it could reach `gh` (Issue #3348), so every
/// non-deadline outcome keeps the historical `main` fallback.
pub fn try_resolve_default_base_branch(
    repo_path: &Path,
) -> Result<String, IssueMonitorScanFailure> {
    ensure_scan_deadline(IssueMonitorScanStage::DefaultBaseBranch)?;
    // Mirrors the gh command root: a workspace home resolves to its child bare
    // repo, anything else runs where the caller pointed us.
    let git_root = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Git,
        "git",
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        gwt_core::process_console::SpawnOptions::new("git symbolic-ref origin/HEAD")
            .current_dir(&git_root),
    );
    if let Err(error) = &output {
        if error.kind() == std::io::ErrorKind::TimedOut {
            return Err(IssueMonitorScanFailure::new(
                IssueMonitorScanStage::DefaultBaseBranch,
                format!("git symbolic-ref origin/HEAD exceeded the scan deadline: {error}"),
            ));
        }
    }
    ensure_scan_deadline(IssueMonitorScanStage::DefaultBaseBranch)?;
    match output {
        Ok(output) if output.success() && !output.stdout.trim().is_empty() => {
            let branch = parse_default_base_branch(&output.stdout);
            if branch.trim().is_empty() {
                Ok(DEFAULT_BASE_BRANCH_FALLBACK.to_string())
            } else {
                Ok(branch)
            }
        }
        _ => Ok(DEFAULT_BASE_BRANCH_FALLBACK.to_string()),
    }
}

/// SPEC #3200 T-041: apply the pre-launch autonomous eligibility gate to every
/// two-stage candidate before the scan claims/launches them. For each candidate
/// it fetches the base-branch protection and runs
/// [`IssueMonitorState::prepare_autonomous_candidate`], which escalates
/// ineligible issues to `NeedsHuman` (removing them from the launch queue) and
/// captures the acceptance snapshot + `Implementing` phase for eligible ones.
/// A no-op unless autonomous mode is on (default OFF preserves SPEC #3165).
pub fn apply_autonomous_eligibility(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    now: &str,
) {
    if let Err(error) = try_apply_autonomous_eligibility(monitor, issues, repo_slug, repo_path, now)
    {
        tracing::warn!(error = %error, "issue monitor autonomous eligibility scan failed");
    }
}

pub fn try_apply_autonomous_eligibility(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    now: &str,
) -> Result<(), IssueMonitorScanFailure> {
    if !monitor.config.enabled || !monitor.autonomous_mode() {
        return Ok(());
    }
    // Only fetch branch protection for candidates whose transient-retry backoff
    // window has elapsed (retry_ready) — a backed-off issue is skipped this scan
    // without a network call (SPEC #3200 T-043/FR-029).
    let candidates: Vec<&IssueMonitorIssue> = issues
        .iter()
        .filter(|issue| monitor.is_autonomous_two_stage_candidate(issue))
        .filter(|issue| monitor.retry_ready(issue.number, now))
        .collect();
    if candidates.is_empty() {
        return Ok(());
    }
    let base_branch = try_resolve_default_base_branch(repo_path)?;
    let protection = run_scan_stage(IssueMonitorScanStage::BranchProtection, || {
        gwt_git::branch_protection::try_fetch_branch_protection(repo_slug, &base_branch)
    })?;
    for issue in candidates {
        let _ = monitor.prepare_autonomous_candidate(issue, &protection, now);
    }
    Ok(())
}

/// SPEC #3200 Option A (daemon-direct + token): advance every in-flight
/// autonomous issue one step through the loop, using freshly-fetched signals.
///
/// - **Implementing** → detect the implementation agent's open PR; on discovery
///   bind it (`begin_review`) and emit a `review_dispatch` so the GUI spawns the
///   independent review agent.
/// - **Reviewing** → once the verdict has arrived, assemble the strong-gate
///   inputs and route: `Deliver` arms the auto-merge (after minting an audit
///   token); `Remediate` re-queues (bounded); `Escalate` → NeedsHuman; `WaitForCi`
///   waits.
/// - **Delivering** → watch for the merge; on merge verify `merged_sha ==
///   reviewed_sha` (TOCTOU layer-4) before completing, else escalate.
///
/// No-op unless autonomous mode is on. Review-dispatch requests are queued on the
/// monitor ([`IssueMonitorState::take_pending_review_dispatches`]) for the GUI to
/// spawn the review agents.
pub fn advance_autonomous_in_flight(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    daemon_secret: &[u8],
    now: &str,
) {
    if let Err(error) =
        try_advance_autonomous_in_flight(monitor, issues, repo_slug, repo_path, daemon_secret, now)
    {
        tracing::warn!(error = %error, "issue monitor autonomous progress scan failed");
    }
}

pub fn try_advance_autonomous_in_flight(
    monitor: &mut IssueMonitorState,
    issues: &[IssueMonitorIssue],
    repo_slug: &str,
    repo_path: &Path,
    daemon_secret: &[u8],
    now: &str,
) -> Result<(), IssueMonitorScanFailure> {
    if !monitor.config.enabled || !monitor.autonomous_mode() {
        return Ok(());
    }
    let base_branch = try_resolve_default_base_branch(repo_path)?;
    for issue_number in monitor.autonomous_in_flight_issues() {
        let Some(record) = monitor.autonomous_record(issue_number).cloned() else {
            continue;
        };
        match record.phase {
            crate::AutonomousPhase::Implementing => {
                let Some(branch) = monitor
                    .inbox_item(issue_number)
                    .and_then(|item| item.launch_plan.as_ref())
                    .map(|plan| plan.branch_name.clone())
                else {
                    continue;
                };
                if let Some(pr) = run_scan_stage(IssueMonitorScanStage::OpenPrReadback, || {
                    gwt_git::pr_status::try_fetch_open_pr_number_for_branch(repo_path, &branch)
                })? {
                    if let Some(sha) =
                        run_scan_stage(IssueMonitorScanStage::HeadShaReadback, || {
                            gwt_git::pr_status::try_fetch_pr_head_sha(repo_path, pr)
                        })?
                    {
                        monitor.begin_review(issue_number, pr, &sha);
                        let criteria = issues
                            .iter()
                            .find(|issue| issue.number == issue_number)
                            .and_then(|issue| issue.body.clone())
                            .map(|body| {
                                crate::issue_monitor_gate::classify_acceptance_criteria(&body).ids
                            })
                            .unwrap_or_default();
                        let diff = run_scan_stage(IssueMonitorScanStage::PrDiffReadback, || {
                            gwt_git::pr_status::try_fetch_pr_diff(repo_path, pr, 200_000)
                        })?
                        .unwrap_or_default();
                        let linked_issue_kind = issues
                            .iter()
                            .find(|issue| issue.number == issue_number)
                            .map(crate::issue_monitor::issue_monitor_linked_issue_kind)
                            .unwrap_or_default();
                        monitor.push_review_dispatch(crate::AutonomousReviewDispatch {
                            issue_number,
                            pr_number: pr,
                            reviewed_sha: sha,
                            required_criteria: criteria,
                            diff,
                            linked_issue_kind,
                        });
                    }
                }
            }
            crate::AutonomousPhase::Reviewing => {
                let Some(pr) = record.pr_number else { continue };
                let protection = run_scan_stage(IssueMonitorScanStage::BranchProtection, || {
                    gwt_git::branch_protection::try_fetch_branch_protection(repo_slug, &base_branch)
                })?;
                let rollup = run_scan_stage(IssueMonitorScanStage::StatusCheckReadback, || {
                    gwt_git::pr_status::try_fetch_pr_status_check_rollup(repo_path, pr)
                })?;
                let head = run_scan_stage(IssueMonitorScanStage::HeadShaReadback, || {
                    gwt_git::pr_status::try_fetch_pr_head_sha(repo_path, pr)
                })?
                .unwrap_or_default();
                let body = issues
                    .iter()
                    .find(|issue| issue.number == issue_number)
                    .and_then(|issue| issue.body.clone())
                    .unwrap_or_default();
                let Some(inputs) =
                    monitor.autonomous_gate_inputs(issue_number, protection, &rollup, &head, &body)
                else {
                    continue; // verdict not back yet → wait
                };
                match crate::issue_monitor_gate::route_autonomous_gate(&inputs) {
                    crate::issue_monitor_gate::GateAction::Deliver => {
                        // Audit: a daemon-signed authorization record bound to the
                        // reviewed SHA (control-plane proof the gate authorized it).
                        let token = crate::issue_monitor_authz::sign_merge_authorization(
                            daemon_secret,
                            issue_number,
                            &inputs.reviewed_sha,
                            &base_branch,
                        );
                        tracing::info!(
                            issue = issue_number,
                            pr,
                            reviewed_sha = %inputs.reviewed_sha,
                            token = %token,
                            "autonomous gate PASS — preparing auto-merge authority"
                        );
                        // The scan is a side-effect-free proposal builder. It
                        // must never submit `gh pr merge --auto` from a cloned
                        // monitor because the daemon may reject this result as
                        // stale after a concurrent OFF/control transition. The
                        // driver persists Prepared, fences Attempting in a
                        // second transaction, then the serialized executor owns
                        // the remote mutation and result reconciliation.
                        let epoch = monitor.effect_authority_epoch();
                        monitor.prepare_effect(crate::PendingIssueMonitorEffect {
                            effect_id: format!(
                                "arm:{issue_number}:{pr}:{}:{epoch}",
                                inputs.reviewed_sha
                            ),
                            authority_epoch: epoch,
                            attempt: 0,
                            state: crate::IssueMonitorEffectState::Prepared,
                            payload: crate::IssueMonitorEffectPayload::ArmAutoMerge {
                                issue_number,
                                pr_number: pr,
                                reviewed_sha: inputs.reviewed_sha,
                            },
                        });
                    }
                    crate::issue_monitor_gate::GateAction::WaitForCi => {}
                    crate::issue_monitor_gate::GateAction::Remediate(reason) => {
                        monitor.record_autonomous_failure(
                            issue_number,
                            crate::FailureClass::Transient,
                            reason,
                            now,
                        );
                    }
                    crate::issue_monitor_gate::GateAction::Escalate(reason) => {
                        monitor.escalate_to_needs_human(issue_number, reason);
                    }
                }
            }
            crate::AutonomousPhase::Delivering => {
                let Some(pr) = record.pr_number else { continue };
                // Merge completion is detected by the presence of a merge commit.
                // The layer-4 identity check then compares the reviewed SHA to the
                // PR's HEAD SHA (`headRefOid`) — NOT the merge commit oid: a squash
                // / merge-commit produces a NEW oid, while `headRefOid` is the head
                // tip that was actually merged (== reviewed SHA when HEAD did not
                // advance). Live-verified against real GitHub (SPEC #3200 layer-4).
                if run_scan_stage(IssueMonitorScanStage::MergeCommitReadback, || {
                    gwt_git::pr_status::try_fetch_pr_merge_commit_sha(repo_path, pr)
                })?
                .is_some()
                {
                    let reviewed = record.reviewed_sha.clone().unwrap_or_default();
                    let merged_head =
                        run_scan_stage(IssueMonitorScanStage::HeadShaReadback, || {
                            gwt_git::pr_status::try_fetch_pr_head_sha(repo_path, pr)
                        })?
                        .unwrap_or_default();
                    if crate::issue_monitor_authz::merged_sha_matches_reviewed(
                        &reviewed,
                        &merged_head,
                    ) {
                        monitor.record_merged(issue_number);
                    } else {
                        tracing::error!(
                            issue = issue_number,
                            reviewed_sha = %reviewed,
                            merged_head = %merged_head,
                            "SECURITY: merged head SHA != reviewed SHA — escalating"
                        );
                        monitor.escalate_to_needs_human(
                            issue_number,
                            "merged head SHA does not match the reviewed SHA",
                        );
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

pub fn load_cached_issue_monitor_candidates(
    cache_root: &Path,
) -> Result<Vec<IssueMonitorIssue>, String> {
    if !cache_root.is_dir() {
        return Ok(Vec::new());
    }
    let cache = Cache::new(cache_root.to_path_buf());
    let mut issues = cache
        .list_entries()
        .map_err(|error| error.to_string())?
        .into_iter()
        .map(|entry| IssueMonitorIssue {
            number: entry.snapshot.number.0,
            title: entry.snapshot.title,
            labels: entry.snapshot.labels,
            state: match entry.snapshot.state {
                IssueState::Open => IssueMonitorIssueState::Open,
                IssueState::Closed => IssueMonitorIssueState::Closed,
            },
            body: (!entry.snapshot.body.is_empty()).then_some(entry.snapshot.body),
            url: None,
        })
        .collect::<Vec<_>>();
    issues.sort_by_key(|issue| issue.number);
    Ok(issues)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubRemoteResolutionError {
    CommandSpawnFailed(String),
    GitCommandFailed {
        status_code: Option<i32>,
        stderr: String,
    },
    OriginNotConfigured(String),
    NonGitHubOrigin(String),
    InvalidGitHubOrigin(String),
}

impl fmt::Display for GitHubRemoteResolutionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CommandSpawnFailed(error) => {
                write!(f, "git remote get-url origin could not be started: {error}")
            }
            Self::GitCommandFailed {
                status_code,
                stderr,
            } => {
                let status = status_code
                    .map(|code| code.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                write!(
                    f,
                    "git remote get-url origin failed with exit status {status}: {stderr}"
                )
            }
            Self::OriginNotConfigured(detail) => {
                write!(f, "Git origin remote is not configured: {detail}")
            }
            Self::NonGitHubOrigin(remote_url) => {
                write!(f, "Git origin remote is not a GitHub URL: {remote_url}")
            }
            Self::InvalidGitHubOrigin(remote_url) => {
                write!(f, "GitHub origin remote URL is invalid: {remote_url}")
            }
        }
    }
}

impl std::error::Error for GitHubRemoteResolutionError {}

pub fn github_remote_owner_and_repo(
    repo_path: &Path,
) -> Result<(String, String), GitHubRemoteResolutionError> {
    github_remote_owner_and_repo_with_program(repo_path, "git")
}

fn github_remote_owner_and_repo_with_program(
    repo_path: &Path,
    program: impl Into<std::ffi::OsString>,
) -> Result<(String, String), GitHubRemoteResolutionError> {
    let git_root = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let output = gwt_core::process_console::spawn_logged_blocking(
        &gwt_core::process_console::global(),
        gwt_core::process_console::ProcessKind::Git,
        program,
        &["remote", "get-url", "origin"],
        gwt_core::process_console::SpawnOptions::new("git remote get-url origin")
            .current_dir(&git_root)
            .forward_output(false),
    )
    .map_err(|error| GitHubRemoteResolutionError::CommandSpawnFailed(error.to_string()))?;
    github_remote_owner_and_repo_from_get_url_output(
        output.success(),
        output.exit_code,
        &output.stdout,
        &output.stderr,
    )
}

pub fn parse_github_remote_url(remote_url: &str) -> Option<(String, String)> {
    let path = remote_url
        .strip_prefix("https://github.com/")
        .or_else(|| remote_url.strip_prefix("http://github.com/"))
        .or_else(|| remote_url.strip_prefix("git@github.com:"))
        .or_else(|| remote_url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let (owner, repo) = path.split_once('/')?;
    if owner.is_empty() || repo.is_empty() || repo.contains('/') {
        return None;
    }
    Some((owner.to_string(), repo.to_string()))
}

fn github_remote_owner_and_repo_from_get_url_output(
    success: bool,
    status_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<(String, String), GitHubRemoteResolutionError> {
    let stdout = stdout.trim();
    let stderr = cleaned_process_text(stderr);
    if !success {
        if stderr.to_ascii_lowercase().contains("no such remote") && stderr.contains("origin") {
            return Err(GitHubRemoteResolutionError::OriginNotConfigured(stderr));
        }
        return Err(GitHubRemoteResolutionError::GitCommandFailed {
            status_code,
            stderr,
        });
    }
    if stdout.is_empty() {
        return Err(GitHubRemoteResolutionError::OriginNotConfigured(
            "git remote get-url origin returned an empty URL".to_string(),
        ));
    }
    if let Some(owner_repo) = parse_github_remote_url(stdout) {
        return Ok(owner_repo);
    }
    if has_supported_github_remote_prefix(stdout) {
        return Err(GitHubRemoteResolutionError::InvalidGitHubOrigin(
            stdout.to_string(),
        ));
    }
    Err(GitHubRemoteResolutionError::NonGitHubOrigin(
        stdout.to_string(),
    ))
}

fn cleaned_process_text(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        "no stderr".to_string()
    } else {
        trimmed.to_string()
    }
}

fn has_supported_github_remote_prefix(remote_url: &str) -> bool {
    [
        "https://github.com/",
        "http://github.com/",
        "git@github.com:",
        "ssh://git@github.com/",
    ]
    .iter()
    .any(|prefix| remote_url.starts_with(prefix))
}

#[allow(dead_code)]
fn _assert_inbox_item_is_send_sync(_: IssueMonitorInboxItem) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{IssueMonitorConfig, MonitorInboxState};
    use gwt_github::{Cache, FakeIssueClient, IssueNumber, IssueSnapshot, IssueState, UpdatedAt};
    use std::path::PathBuf;

    fn issue(number: u64) -> IssueMonitorIssue {
        IssueMonitorIssue {
            number,
            title: format!("Issue {number}"),
            labels: vec!["auto-improve".to_string()],
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
        }
    }

    fn github_issue(number: u64) -> IssueSnapshot {
        IssueSnapshot {
            number: IssueNumber(number),
            title: format!("Issue {number}"),
            body: String::new(),
            labels: vec![],
            state: IssueState::Open,
            updated_at: UpdatedAt::new("t1"),
            comments: vec![],
        }
    }

    #[test]
    fn payloads_surface_autonomous_notices_as_toasts_when_gui_is_connected() {
        // SPEC #3200 FR-034 (T-111): daemon-side autonomous transitions queue
        // operator notices; the worker drains them into `toast` payloads so the
        // GUI's issue_monitor_toast pipe (surface toast + persistent autonomous
        // notification stack) receives them.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.set_gui_connected(true);
        monitor.record_attempt(42);
        monitor.escalate_to_needs_human(42, "review rejected");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);

        let toast = payloads
            .iter()
            .find(|payload| {
                payload.event == "toast"
                    && payload.payload.get("issue_number").and_then(|v| v.as_u64()) == Some(42)
            })
            .expect("autonomous notice surfaces as a toast payload");
        assert_eq!(
            toast.payload.get("level").and_then(|v| v.as_str()),
            Some("error")
        );
        assert!(toast
            .payload
            .get("message")
            .and_then(|v| v.as_str())
            .is_some_and(|message| message.contains("review rejected")));
        // Drained: a second pass emits no duplicate.
        let again = issue_monitor_daemon_payloads(&mut monitor, true);
        assert!(!again.iter().any(|payload| {
            payload.event == "toast"
                && payload.payload.get("issue_number").and_then(|v| v.as_u64()) == Some(42)
        }));
    }

    #[test]
    fn payloads_retain_autonomous_notices_while_no_gui_is_connected() {
        // Fully-unattended window: with no GUI connected the notices stay queued
        // (bounded) instead of being dropped, and surface on the next connect.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig {
                enabled: true,
                ..IssueMonitorConfig::default()
            },
            crate::IssueMonitorPrefs {
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        monitor.record_attempt(42);
        monitor.escalate_to_needs_human(42, "boom");

        let offline = issue_monitor_daemon_payloads(&mut monitor, false);
        assert!(
            !offline.iter().any(|payload| payload.event == "toast"
                && payload.payload.get("issue_number").and_then(|v| v.as_u64()) == Some(42)),
            "no toast emitted while the GUI is disconnected"
        );

        let online = issue_monitor_daemon_payloads(&mut monitor, true);
        assert!(
            online.iter().any(|payload| payload.event == "toast"
                && payload.payload.get("issue_number").and_then(|v| v.as_u64()) == Some(42)),
            "queued notice surfaces once a GUI connects"
        );
    }

    #[test]
    fn payloads_surface_stalled_scan_through_status_error() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            poll_interval_secs: 10,
            ..IssueMonitorConfig::default()
        });
        crate::scan_issue_monitor_candidates(&mut monitor, &[], "2000-01-01T00:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, false);

        let status = payloads
            .iter()
            .find(|payload| payload.event == "status")
            .expect("status payload");
        assert_eq!(status.payload["state"], "error");
        assert_eq!(
            status.payload["last_error"],
            "Issue Monitor scan stalled; last scan at 2000-01-01T00:00:00Z"
        );
        assert_eq!(status.payload["last_scan_at"], "2000-01-01T00:00:00Z");
    }

    #[test]
    fn claim_skips_and_marks_issues_already_completed_by_a_merged_pr() {
        // Issue #3225: an issue whose fix is already merged (a linked PR in
        // MERGED state) must not be re-launched by a fresh monitor — the
        // completion signal must come from GitHub, not instance-local prefs.
        // The claim loop probes right before claiming; positives are recorded
        // Merged (persisted) and the slot goes to the next queued candidate.
        let mut monitor = IssueMonitorState::new(crate::IssueMonitorConfig {
            enabled: true,
            max_active: 1,
            ..crate::IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        crate::scan_issue_monitor_candidates(
            &mut monitor,
            &[issue(42), issue(43)],
            "2026-07-02T00:00:00Z",
        );
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        client.seed(github_issue(43));

        // #42 is already completed by a merged PR; #43 is genuinely open work.
        let launches = monitor.claim_next_launch_requests_with_probe(
            &client,
            "host:1",
            "2026-07-02T00:00:10Z",
            1,
            |issue_number| issue_number == 42,
        );

        assert_eq!(
            launches.iter().map(|l| l.issue_number).collect::<Vec<_>>(),
            vec![43],
            "the completed issue is skipped; the slot goes to real work"
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(crate::MonitorInboxState::Merged),
            "completed issue is recorded Merged (persisted, never relaunched)"
        );
        assert!(monitor.prefs().merged_issues.contains(&42));
        // Idempotent on later scans: stays Merged, never re-queued.
        crate::scan_issue_monitor_candidates(&mut monitor, &[issue(42)], "2026-07-02T00:01:00Z");
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(crate::MonitorInboxState::Merged)
        );
    }

    #[test]
    fn payloads_keep_queue_when_no_gui_is_connected() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_claimed(issue(42), "claim-a");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, false);

        assert!(payloads.iter().any(|payload| payload.event == "status"));
        assert!(payloads.iter().any(|payload| payload.event == "inbox"));
        assert!(!payloads
            .iter()
            .any(|payload| payload.event == "launch_request"));
        assert_eq!(monitor.queue_len(), 1);
        assert_eq!(monitor.active_issue_number(), None);
    }

    #[test]
    fn read_only_payloads_do_not_drain_recovery_blocked_deliveries() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));

        let read_only = issue_monitor_read_only_daemon_payloads(&monitor);

        assert_eq!(
            read_only
                .iter()
                .map(|payload| payload.event.as_str())
                .collect::<Vec<_>>(),
            vec!["status", "inbox"],
        );
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 1);

        let after_recovery = issue_monitor_daemon_payloads(&mut monitor, true);
        assert!(after_recovery.iter().any(|payload| {
            payload.event == "launch_request"
                && payload.payload["delivery_id"] == "launch:effect-42"
        }));
    }

    #[test]
    fn payloads_emit_launch_request_when_gui_is_connected() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);

        assert!(payloads.iter().any(|payload| {
            payload.event == "launch_request" && payload.payload["issue_number"] == 42
        }));
        assert_eq!(monitor.active_issue_number(), Some(42));
        assert_eq!(
            monitor.inbox_item(42).expect("inbox item").state,
            MonitorInboxState::Launching
        );
    }

    #[test]
    fn durable_launch_delivery_survives_fanout_zero_and_replays_one_stable_identity() {
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.record_candidate(issue(42));
        assert!(monitor.apply_confirmed_claim(
            42,
            "claim-42",
            "host/session",
            "effect-42",
            "2026-07-28T00:00:00Z",
        ));

        let offline = issue_monitor_daemon_payloads(&mut monitor, false);
        assert!(!offline
            .iter()
            .any(|payload| payload.event == "launch_request"));
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 1);

        let first = issue_monitor_daemon_payloads(&mut monitor, true);
        let replay = issue_monitor_daemon_payloads(&mut monitor, true);
        for payloads in [&first, &replay] {
            let launch = payloads
                .iter()
                .find(|payload| payload.event == "launch_request")
                .expect("durable launch request");
            assert_eq!(launch.payload["delivery_id"], "launch:effect-42");
        }
        assert_eq!(monitor.prefs().pending_launch_deliveries.len(), 1);
    }

    #[test]
    fn payloads_emit_launch_request_before_launching_snapshot_when_gui_is_connected() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);
        let launch_index = payloads
            .iter()
            .position(|payload| payload.event == "launch_request")
            .expect("launch request payload");
        let first_status_index = payloads
            .iter()
            .position(|payload| payload.event == "status")
            .expect("status payload");

        assert!(
            launch_index < first_status_index,
            "the agent window launch request must reach the GUI before the monitor renders Launching"
        );
    }

    #[test]
    fn payloads_emit_all_pending_launch_requests_when_parallel_capacity_allows() {
        let client = FakeIssueClient::new();
        client.seed(github_issue(42));
        client.seed(github_issue(43));
        client.seed(github_issue(44));
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig {
            enabled: true,
            max_active: 3,
            ..IssueMonitorConfig::default()
        });
        monitor.set_gui_connected(true);
        monitor.record_candidate(issue(42));
        monitor.record_candidate(issue(43));
        monitor.record_candidate(issue(44));
        monitor.claim_next_launch_requests(&client, "host-a/session-a", "2026-06-23T10:00:00Z");

        let payloads = issue_monitor_daemon_payloads(&mut monitor, true);
        let launch_numbers: Vec<u64> = payloads
            .iter()
            .filter(|payload| payload.event == "launch_request")
            .filter_map(|payload| payload.payload["issue_number"].as_u64())
            .collect();

        assert_eq!(launch_numbers, vec![42, 43, 44]);
        assert_eq!(monitor.active_count(), 3);
    }

    #[test]
    fn cached_issue_candidates_load_from_issue_cache_entries() {
        let dir = tempfile::tempdir().expect("tempdir");
        let cache = Cache::new(dir.path().to_path_buf());
        let mut spec = github_issue(3165);
        spec.title = "SPEC: Issue auto-improve monitor".to_string();
        spec.labels = vec!["gwt-spec".to_string()];
        let mut closed = github_issue(3000);
        closed.title = "Closed issue".to_string();
        closed.state = IssueState::Closed;
        cache.write_snapshot(&spec).expect("write spec");
        cache.write_snapshot(&closed).expect("write closed issue");

        let candidates = load_cached_issue_monitor_candidates(dir.path()).expect("load cache");

        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].number, 3000);
        assert_eq!(candidates[0].state, IssueMonitorIssueState::Closed);
        assert_eq!(candidates[1].number, 3165);
        assert_eq!(candidates[1].title, "SPEC: Issue auto-improve monitor");
        assert_eq!(candidates[1].labels, vec!["gwt-spec"]);
        assert_eq!(candidates[1].state, IssueMonitorIssueState::Open);
    }

    #[test]
    fn loaded_candidate_provenance_distinguishes_live_success_from_cache_fallback() {
        let live_issue = issue(42);
        let cached_issue = issue(43);

        let live = resolve_loaded_issue_monitor_candidates(
            Ok(vec![live_issue.clone()]),
            [Ok(vec![cached_issue.clone()])],
        )
        .expect("live result");
        assert_eq!(live.source, IssueMonitorCandidateSource::Live);
        assert!(live.authorizes_remote_effects());
        assert_eq!(live.live_error, None);
        assert_eq!(live.issues, vec![live_issue]);

        let empty_live = resolve_loaded_issue_monitor_candidates(
            Ok(Vec::new()),
            [Ok(vec![cached_issue.clone()])],
        )
        .expect("empty live result still authoritative");
        assert_eq!(empty_live.source, IssueMonitorCandidateSource::Live);
        assert!(empty_live.authorizes_remote_effects());
        assert!(empty_live.issues.is_empty());

        let limit_sized_live = resolve_loaded_issue_monitor_candidates(
            Ok((1..=1_000).map(issue).collect()),
            std::iter::empty::<Result<Vec<IssueMonitorIssue>, String>>(),
        )
        .expect("limit-sized live result");
        assert_eq!(
            limit_sized_live.source,
            IssueMonitorCandidateSource::LiveIncomplete,
            "a capped gh list cannot prove that an absent in-flight Issue no longer exists"
        );

        let cache = resolve_loaded_issue_monitor_candidates(
            Err("gh unavailable".to_string()),
            [Ok(Vec::new()), Ok(vec![cached_issue.clone()])],
        )
        .expect("cache fallback");
        assert_eq!(cache.source, IssueMonitorCandidateSource::Cache);
        assert!(!cache.authorizes_remote_effects());
        assert_eq!(cache.live_error.as_deref(), Some("gh unavailable"));
        assert_eq!(cache.issues, vec![cached_issue]);

        let error = resolve_loaded_issue_monitor_candidates(
            Err("gh unavailable".to_string()),
            [Ok(Vec::new()), Err("cache corrupt".to_string())],
        )
        .expect_err("no usable cache preserves live error");
        assert_eq!(error, "gh unavailable");
    }

    #[test]
    fn cached_candidates_preserve_live_scan_error_after_read_model_refresh() {
        let dir = tempfile::TempDir::new().expect("tempdir");
        let mut monitor = IssueMonitorState::new(crate::IssueMonitorConfig::default());
        let loaded = LoadedIssueMonitorCandidates {
            issues: vec![issue(42)],
            source: IssueMonitorCandidateSource::Cache,
            live_error: Some("operation deadline exceeded at issue-list stage".to_string()),
        };

        let summary = scan_loaded_issue_monitor_candidates(
            &mut monitor,
            &loaded,
            dir.path(),
            "2026-07-27T00:00:00Z",
        );

        assert_eq!(summary.scanned, 1);
        assert_eq!(
            monitor.status_view().last_error.as_deref(),
            Some(
                "issue list failed; using cache fallback: operation deadline exceeded at issue-list stage"
            )
        );
    }

    #[test]
    fn parse_github_remote_url_accepts_https_and_ssh_forms() {
        assert_eq!(
            parse_github_remote_url("https://github.com/owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_remote_url("git@github.com:owner/repo.git"),
            Some(("owner".to_string(), "repo".to_string()))
        );
        assert_eq!(
            parse_github_remote_url("https://example.com/owner/repo"),
            None
        );
    }

    #[test]
    fn proposal_return_deadline_expiry_is_stage_typed() {
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() - std::time::Duration::from_millis(1),
        );

        let error = ensure_scan_deadline(IssueMonitorScanStage::ProposalReturn)
            .expect_err("expired final deadline must reject the proposal");

        assert_eq!(error.stage, IssueMonitorScanStage::ProposalReturn);
        assert!(error.to_string().contains("proposal-return"));
    }

    #[test]
    fn github_remote_output_resolves_valid_origin() {
        assert_eq!(
            github_remote_owner_and_repo_from_get_url_output(
                true,
                Some(0),
                "https://github.com/owner/repo.git\n",
                ""
            )
            .expect("valid origin"),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[test]
    fn github_remote_owner_and_repo_accepts_workspace_home_with_child_bare_repo() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let bare_repo_path = tmp.path().join("gwt.git");
        let status = gwt_core::process::hidden_command("git")
            .args(["init", "--bare"])
            .arg(&bare_repo_path)
            .status()
            .expect("git init --bare");
        assert!(status.success(), "git init --bare failed");
        let status = gwt_core::process::hidden_command("git")
            .args([
                "remote",
                "add",
                "origin",
                "https://github.com/owner/repo.git",
            ])
            .current_dir(&bare_repo_path)
            .status()
            .expect("git remote add origin");
        assert!(status.success(), "git remote add origin failed");

        assert_eq!(
            github_remote_owner_and_repo(tmp.path()).expect("workspace home origin"),
            ("owner".to_string(), "repo".to_string())
        );
    }

    #[cfg(unix)]
    #[test]
    fn github_remote_owner_and_repo_stops_hanging_program_at_operation_deadline() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().expect("tempdir");
        let fake_git = temp.path().join("git");
        std::fs::write(
            &fake_git,
            r#"#!/bin/sh
if [ "$1" = "remote" ] && [ "$2" = "get-url" ] && [ "$3" = "origin" ]; then
  sleep 2
  printf '%s\n' 'https://github.com/owner/repo.git'
  exit 0
fi
exit 1
"#,
        )
        .expect("write fake git");
        std::fs::set_permissions(&fake_git, std::fs::Permissions::from_mode(0o755))
            .expect("make fake git executable");
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo path");
        let started = std::time::Instant::now();
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            started + std::time::Duration::from_millis(150),
        );

        let error = github_remote_owner_and_repo_with_program(&repo, fake_git.as_os_str())
            .expect_err("ambient deadline must stop the hanging git remote lookup");

        assert!(error.to_string().contains("deadline"), "{error}");
        assert!(
            started.elapsed() < std::time::Duration::from_millis(1_500),
            "hanging git outlived the deadline: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn github_remote_output_classifies_missing_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            false,
            Some(2),
            "",
            "error: No such remote 'origin'\n",
        )
        .expect_err("missing origin");

        assert_eq!(
            error.to_string(),
            "Git origin remote is not configured: error: No such remote 'origin'"
        );
    }

    #[test]
    fn github_remote_output_classifies_git_failure() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            false,
            Some(128),
            "",
            "fatal: not a git repository\n",
        )
        .expect_err("git failure");

        assert_eq!(
            error.to_string(),
            "git remote get-url origin failed with exit status 128: fatal: not a git repository"
        );
    }

    #[test]
    fn github_remote_output_classifies_non_github_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            true,
            Some(0),
            "https://example.com/owner/repo.git\n",
            "",
        )
        .expect_err("non GitHub origin");

        assert_eq!(
            error.to_string(),
            "Git origin remote is not a GitHub URL: https://example.com/owner/repo.git"
        );
    }

    #[test]
    fn github_remote_output_classifies_invalid_github_origin() {
        let error = github_remote_owner_and_repo_from_get_url_output(
            true,
            Some(0),
            "https://github.com/owner\n",
            "",
        )
        .expect_err("invalid GitHub origin");

        assert_eq!(
            error.to_string(),
            "GitHub origin remote URL is invalid: https://github.com/owner"
        );
    }

    #[test]
    fn apply_autonomous_eligibility_is_noop_when_mode_off() {
        // SPEC #3200 FR-001: default autonomous_mode OFF ⇒ no autonomous state is
        // created and (crucially) no branch-protection network call is made. The
        // early return runs before any gh invocation, so this test exercises the
        // gate without touching the network.
        use crate::{
            IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState, IssueMonitorState,
        };
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        let issues = vec![IssueMonitorIssue {
            number: 50,
            title: "t".to_string(),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: Some("## Acceptance Criteria\n- [ ] AC-1: x\n".to_string()),
            url: None,
        }];
        apply_autonomous_eligibility(
            &mut monitor,
            &issues,
            "owner/repo",
            std::path::Path::new("/tmp/repo"),
            "2026-06-29T00:00:00Z",
        );
        assert!(
            monitor.autonomous_record(50).is_none(),
            "off ⇒ no autonomous state created, no network call",
        );
    }

    #[test]
    fn advance_autonomous_in_flight_is_noop_when_mode_off() {
        // Default OFF ⇒ no phase advancement, no network call, no merge.
        use crate::{
            AutonomousPhase, IssueMonitorConfig, IssueMonitorIssue, IssueMonitorIssueState,
            IssueMonitorState,
        };
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_autonomous_phase(50, AutonomousPhase::Reviewing); // would otherwise act
        let issues = vec![IssueMonitorIssue {
            number: 50,
            title: "t".to_string(),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: None,
            url: None,
        }];
        advance_autonomous_in_flight(
            &mut monitor,
            &issues,
            "owner/repo",
            std::path::Path::new("/tmp/repo"),
            b"secret",
            "2026-06-29T00:00:00Z",
        );
        assert_eq!(
            monitor.autonomous_record(50).map(|r| r.phase),
            Some(AutonomousPhase::Reviewing),
            "off ⇒ phase unchanged, no network/merge",
        );
        assert!(monitor.take_pending_review_dispatches().is_empty());
    }

    #[test]
    fn advance_autonomous_in_flight_is_noop_when_global_monitor_is_disabled() {
        use crate::{AutonomousPhase, IssueMonitorConfig, IssueMonitorState};
        let mut monitor = IssueMonitorState::new(IssueMonitorConfig::default());
        monitor.set_autonomous_mode(true);
        monitor.set_autonomous_phase(50, AutonomousPhase::Reviewing);

        advance_autonomous_in_flight(
            &mut monitor,
            &[],
            "owner/repo",
            std::path::Path::new("/tmp/repo"),
            b"secret",
            "2026-06-29T00:00:00Z",
        );

        assert_eq!(
            monitor.autonomous_record(50).map(|record| record.phase),
            Some(AutonomousPhase::Reviewing)
        );
        assert!(monitor.pending_effects().is_empty());
    }

    #[test]
    fn parse_default_base_branch_strips_origin_prefix_and_fails_closed() {
        assert_eq!(parse_default_base_branch("origin/main\n"), "main");
        assert_eq!(parse_default_base_branch("origin/develop"), "develop");
        // A bare name with no origin/ prefix is taken as-is.
        assert_eq!(parse_default_base_branch("trunk"), "trunk");
        // Empty / unresolved ⇒ fail-closed to main.
        assert_eq!(parse_default_base_branch(""), "main");
        assert_eq!(parse_default_base_branch("origin/"), "main");
    }

    #[test]
    fn resolve_default_base_branch_uses_child_bare_repo_for_workspace_home() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let bare_repo = tmp.path().join("repo.git");
        let init = gwt_core::process::hidden_command("git")
            .args([
                "init",
                "--bare",
                bare_repo.to_str().expect("bare repo path"),
            ])
            .current_dir(tmp.path())
            .output()
            .expect("git init --bare");
        assert!(
            init.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );
        let symbolic_ref = gwt_core::process::hidden_command("git")
            .args([
                "--git-dir",
                bare_repo.to_str().expect("bare repo path"),
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/develop",
            ])
            .current_dir(tmp.path())
            .output()
            .expect("set origin HEAD");
        assert!(
            symbolic_ref.status.success(),
            "git symbolic-ref failed: {}",
            String::from_utf8_lossy(&symbolic_ref.stderr)
        );

        assert_eq!(resolve_default_base_branch(tmp.path()), "develop");
    }

    /// Issue #3348: a repository that never configured `origin/HEAD` is a
    /// normal state. Reporting it as a scan failure aborted the whole
    /// autonomous progress loop before it could reach `gh`.
    #[test]
    fn unset_origin_head_falls_back_instead_of_failing_the_scan() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let bare_repo = tmp.path().join("repo.git");
        let init = gwt_core::process::hidden_command("git")
            .args([
                "init",
                "--bare",
                bare_repo.to_str().expect("bare repo path"),
            ])
            .current_dir(tmp.path())
            .output()
            .expect("git init --bare");
        assert!(
            init.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        assert_eq!(
            try_resolve_default_base_branch(tmp.path())
                .expect("unset origin/HEAD is not a failure"),
            DEFAULT_BASE_BRANCH_FALLBACK,
        );
    }

    /// Issue #3349: the scan deadline is still the one condition that aborts
    /// this stage, so a hung `git symbolic-ref` can never freeze the driver.
    #[test]
    fn expired_scan_deadline_still_fails_default_base_resolution() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let _deadline = gwt_core::operation_deadline::ScopedOperationDeadline::enter(
            std::time::Instant::now() - std::time::Duration::from_millis(1),
        );

        let failure = try_resolve_default_base_branch(tmp.path())
            .expect_err("an expired deadline must abort the stage");

        assert_eq!(failure.stage, IssueMonitorScanStage::DefaultBaseBranch);
    }

    /// A `gh` stand-in that refuses to answer unless it was spawned from inside
    /// a bare repository (`HEAD` + `objects/` + `refs/` sit directly in the
    /// cwd). A container root holds none of those, so this is exactly the
    /// failure Issue #3348 observed in production.
    fn write_bare_repo_bound_fake_gh(bin_dir: &Path, call_log: &Path) -> PathBuf {
        std::fs::create_dir_all(bin_dir).expect("create fake gh bin dir");
        let call_log = call_log.display().to_string();
        #[cfg(windows)]
        {
            let fake_gh = bin_dir.join("gh.cmd");
            std::fs::write(
                &fake_gh,
                format!(
                    "@echo off\r\n\
if not exist \"HEAD\" goto notbare\r\n\
if not exist \"objects\\\" goto notbare\r\n\
if not exist \"refs\\\" goto notbare\r\n\
echo %*>>\"{call_log}\"\r\n\
echo %* | findstr /C:\"mergeCommit\" >nul\r\n\
if not errorlevel 1 (\r\n\
  echo {{\"mergeCommit\":{{\"oid\":\"squash999\"}}}}\r\n\
  exit /b 0\r\n\
)\r\n\
echo %* | findstr /C:\"headRefOid\" >nul\r\n\
if not errorlevel 1 (\r\n\
  echo {{\"headRefOid\":\"abc123\"}}\r\n\
  exit /b 0\r\n\
)\r\n\
echo {{}}\r\n\
exit /b 0\r\n\
:notbare\r\n\
>&2 echo gh ran outside the child bare repository: %CD%\r\n\
exit /b 1\r\n"
                ),
            )
            .expect("write fake gh");
            fake_gh
        }
        #[cfg(not(windows))]
        {
            let fake_gh = bin_dir.join("gh");
            std::fs::write(
                &fake_gh,
                format!(
                    r#"#!/bin/sh
if [ ! -f HEAD ] || [ ! -d objects ] || [ ! -d refs ]; then
  printf '%s\n' "gh ran outside the child bare repository: $(pwd)" >&2
  exit 1
fi
printf '%s\n' "$*" >> "{call_log}"
case "$*" in
  *mergeCommit*) printf '%s\n' '{{"mergeCommit":{{"oid":"squash999"}}}}' ;;
  *headRefOid*)  printf '%s\n' '{{"headRefOid":"abc123"}}' ;;
  *)             printf '%s\n' '{{}}' ;;
esac
exit 0
"#
                ),
            )
            .expect("write fake gh");
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake_gh, std::fs::Permissions::from_mode(0o755))
                .expect("chmod fake gh");
            fake_gh
        }
    }

    /// Issue #3348 regression: in a container-layout project the monitored
    /// project root is NOT a git repository — it merely holds the bare
    /// `<repo>.git` plus the worktrees. Every autonomous-loop `gh` call used to
    /// be spawned with that raw root as cwd, so gh could not resolve the base
    /// repo and each fetch failed silently: the loop never advanced, the merge
    /// was never observed, and the active slot stayed held forever. The fake
    /// `gh` here fails unless it runs inside the child bare repo, so reverting
    /// the `main_worktree_root` normalization turns this test red.
    #[test]
    fn advance_autonomous_in_flight_reaches_gh_from_child_bare_repo_for_container_root() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = tempfile::tempdir().expect("tempdir");
        let container_root = tmp.path().join("workspace");
        std::fs::create_dir_all(&container_root).expect("create container root");
        let bare_repo = container_root.join("repo.git");
        let init = gwt_core::process::hidden_command("git")
            .args([
                "init",
                "--bare",
                bare_repo.to_str().expect("bare repo path"),
            ])
            .output()
            .expect("git init --bare");
        assert!(
            init.status.success(),
            "git init --bare failed: {}",
            String::from_utf8_lossy(&init.stderr)
        );

        let call_log = tmp.path().join("gh-calls.log");
        let fake_gh = write_bare_repo_bound_fake_gh(&tmp.path().join("bin"), &call_log);
        let mut path_entries = vec![fake_gh.parent().expect("fake gh parent").to_path_buf()];
        if let Some(existing) = std::env::var_os("PATH") {
            path_entries.extend(std::env::split_paths(&existing));
        }
        let _path = gwt_core::test_support::ScopedEnvVar::set(
            "PATH",
            std::env::join_paths(path_entries).expect("join PATH"),
        );

        let issues = vec![IssueMonitorIssue {
            number: 42,
            title: "Issue 42".to_string(),
            labels: vec!["auto-merge".to_string()],
            state: IssueMonitorIssueState::Open,
            body: Some("## Acceptance Criteria\n- [ ] AC-1: returns 200\n".to_string()),
            url: None,
        }];
        // `enabled` is required as well as `autonomous_mode`: the global kill
        // switch gates every autonomous remote call
        // (`advance_autonomous_in_flight_is_noop_when_global_monitor_is_disabled`),
        // and this test is about the gh command root, not the kill switch.
        let mut monitor = IssueMonitorState::with_prefs(
            IssueMonitorConfig::default(),
            crate::IssueMonitorPrefs {
                enabled: true,
                autonomous_mode: true,
                ..crate::IssueMonitorPrefs::default()
            },
        );
        crate::scan_issue_monitor_candidates(&mut monitor, &issues, "2026-07-28T00:00:00Z");
        // Delivering is the narrowest phase that proves the loop reaches gh: it
        // watches the merge (`--json mergeCommit`) and then re-reads the merged
        // head (`--json headRefOid`) for the layer-4 identity check.
        monitor.begin_review(42, 7, "abc123");
        monitor.record_review_verdict(42, true);
        monitor.begin_delivering(42);

        advance_autonomous_in_flight(
            &mut monitor,
            &issues,
            "owner/repo",
            &container_root,
            b"secret",
            "2026-07-28T00:10:00Z",
        );

        let calls = std::fs::read_to_string(&call_log).unwrap_or_default();
        assert!(
            calls.contains("mergeCommit") && calls.contains("headRefOid"),
            "the loop must reach gh from the child bare repo (calls={calls:?})",
        );
        assert_eq!(
            monitor.inbox_item(42).map(|item| item.state),
            Some(MonitorInboxState::Merged),
            "a merged PR observed through the normalized gh path frees the slot",
        );
        assert_eq!(monitor.active_count(), 0);
    }
}
