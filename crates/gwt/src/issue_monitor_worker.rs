use std::{fmt, path::Path};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    scan_issue_monitor_candidates_with_provenance, IssueMonitorCandidateSource,
    IssueMonitorInboxItem, IssueMonitorIssue, IssueMonitorIssueState, IssueMonitorScanSummary,
    IssueMonitorState,
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
}

pub fn issue_monitor_daemon_payloads(
    monitor: &mut IssueMonitorState,
    gui_connected: bool,
) -> Vec<IssueMonitorDaemonPayload> {
    monitor.set_gui_connected(gui_connected);
    let mut payloads = Vec::new();
    if gui_connected {
        for request in monitor.take_pending_launch_requests() {
            payloads.push(IssueMonitorDaemonPayload {
                event: "launch_request".to_string(),
                payload: serde_json::json!({
                    "issue_number": request.issue_number,
                    "branch_name": request.branch_name,
                    "linked_issue_kind": request.linked_issue_kind,
                }),
            });
            payloads.push(IssueMonitorDaemonPayload {
                event: "toast".to_string(),
                payload: serde_json::json!({
                    "level": "info",
                    "message": "Issue Monitor launch requested",
                    "issue_number": request.issue_number,
                }),
            });
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

    payloads.extend([
        IssueMonitorDaemonPayload {
            event: "status".to_string(),
            payload: serde_json::to_value(monitor.status_view())
                .expect("issue monitor status serializes"),
        },
        IssueMonitorDaemonPayload {
            event: "inbox".to_string(),
            payload: serde_json::to_value(monitor.inbox.clone())
                .expect("issue monitor inbox serializes"),
        },
    ]);

    payloads
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

/// Load a complete live candidate list when available, retaining typed
/// provenance when a live GitHub failure falls back to a cache snapshot. The
/// existing Vec-returning API above remains a compatibility wrapper.
pub fn load_open_issue_monitor_candidates_for_repo_path_with_provenance(
    repo_path: &Path,
    owner: &str,
    repo: &str,
) -> Result<LoadedIssueMonitorCandidates, String> {
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
    resolve_loaded_issue_monitor_candidates(
        load_open_issue_monitor_candidates(owner, repo),
        cache_results,
    )
}

fn resolve_loaded_issue_monitor_candidates<I>(
    live_result: Result<Vec<IssueMonitorIssue>, String>,
    cache_results: I,
) -> Result<LoadedIssueMonitorCandidates, String>
where
    I: IntoIterator<Item = Result<Vec<IssueMonitorIssue>, String>>,
{
    match live_result {
        Ok(issues) => Ok(LoadedIssueMonitorCandidates {
            issues,
            source: IssueMonitorCandidateSource::Live,
        }),
        Err(live_error) => {
            for issues in cache_results.into_iter().flatten() {
                if !issues.is_empty() {
                    return Ok(LoadedIssueMonitorCandidates {
                        issues,
                        source: IssueMonitorCandidateSource::Cache,
                    });
                }
            }
            Err(live_error)
        }
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
    scan_issue_monitor_candidates_with_provenance(
        monitor,
        &loaded.issues,
        loaded.source,
        repo_path,
        now,
    )
}

/// Issue #3225: GitHub-derived completion probe for the claim loop — "does
/// this issue have a linked PR that is already MERGED?". Uses the issue's
/// timeline (cross-referenced / connected PRs), so it catches fixes merged via
/// ANY branch, not just the monitor's own `work/issue-N`. Fails open (false)
/// on errors so a transient gh failure never blocks real work.
pub fn issue_completed_by_merged_pr(owner: &str, repo: &str, issue_number: u64) -> bool {
    match crate::cli::issue::fetch_linked_prs_via_gh(
        owner,
        repo,
        gwt_github::IssueNumber(issue_number),
    ) {
        // codex #3226 review: only a PR that actually CLOSES the issue counts
        // — a merged PR that merely references it (Refs #N / partial work)
        // must not mark the issue done.
        Ok(prs) => prs
            .iter()
            .any(|pr| pr.will_close_target && pr.state.eq_ignore_ascii_case("merged")),
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

/// Mark any active launched Issue whose work branch has a merged PR as
/// `Merged`, freeing the active slot. Skips the network call when nothing is
/// launched, and leaves work launched when the PR query fails (so a transient
/// error never closes the slot on a false signal).
pub fn reconcile_issue_monitor_merges(monitor: &mut IssueMonitorState, repo_path: &Path) {
    if monitor.active_launched_branches().is_empty() {
        return;
    }
    match gwt_git::pr_status::fetch_merged_pr_branches(repo_path) {
        Ok(merged_branches) => {
            let merged = monitor.reconcile_merged_branches(&merged_branches);
            if !merged.is_empty() {
                tracing::info!(
                    issues = ?merged,
                    "issue monitor marked merged work and freed active slots"
                );
            }
        }
        Err(error) => {
            tracing::debug!(
                error = %error,
                "issue monitor merge reconciliation skipped (PR query failed)"
            );
        }
    }
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

/// Resolve the repo's default base branch (the branch autonomous PRs merge
/// into) via `origin/HEAD`. Fail-closed to `main` on any failure.
pub fn resolve_default_base_branch(repo_path: &Path) -> String {
    let hub = gwt_core::process_console::global();
    let output = gwt_core::process_console::spawn_logged_blocking(
        &hub,
        gwt_core::process_console::ProcessKind::Git,
        "git",
        &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"],
        gwt_core::process_console::SpawnOptions::new("git symbolic-ref origin/HEAD")
            .current_dir(repo_path),
    );
    match output {
        Ok(output) if output.success() => parse_default_base_branch(&output.stdout),
        _ => "main".to_string(),
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
    if !monitor.autonomous_mode() {
        return;
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
        return;
    }
    let base_branch = resolve_default_base_branch(repo_path);
    let protection = gwt_git::branch_protection::fetch_branch_protection(repo_slug, &base_branch);
    for issue in candidates {
        let _ = monitor.prepare_autonomous_candidate(issue, &protection, now);
    }
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
    if !monitor.autonomous_mode() {
        return;
    }
    let base_branch = resolve_default_base_branch(repo_path);
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
                if let Some(pr) =
                    gwt_git::pr_status::fetch_open_pr_number_for_branch(repo_path, &branch)
                {
                    if let Some(sha) = gwt_git::pr_status::fetch_pr_head_sha(repo_path, pr) {
                        monitor.begin_review(issue_number, pr, &sha);
                        let criteria = issues
                            .iter()
                            .find(|issue| issue.number == issue_number)
                            .and_then(|issue| issue.body.clone())
                            .map(|body| {
                                crate::issue_monitor_gate::classify_acceptance_criteria(&body).ids
                            })
                            .unwrap_or_default();
                        let diff = gwt_git::pr_status::fetch_pr_diff(repo_path, pr, 200_000)
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
                let protection =
                    gwt_git::branch_protection::fetch_branch_protection(repo_slug, &base_branch);
                let rollup = gwt_git::pr_status::fetch_pr_status_check_rollup(repo_path, pr);
                let head = gwt_git::pr_status::fetch_pr_head_sha(repo_path, pr).unwrap_or_default();
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
                            "autonomous gate PASS — arming auto-merge"
                        );
                        monitor.begin_delivering(issue_number);
                        // Bind the arm to the reviewed head SHA so GitHub refuses
                        // to merge if the head advanced past what the gate reviewed.
                        // codex #3217 review: announce the arm ONLY on success —
                        // a failed arm must not leave a success toast while the
                        // record would otherwise sit in Delivering with nothing
                        // armed. Fail closed: escalate so the operator acts.
                        if gwt_git::pr_status::merge_pr_auto(repo_path, pr, &inputs.reviewed_sha) {
                            monitor.record_auto_merge_armed(issue_number);
                        } else {
                            tracing::warn!(issue = issue_number, pr, "auto-merge arm failed");
                            monitor.escalate_to_needs_human(
                                issue_number,
                                "auto-merge arming failed after gate pass — arm manually or relaunch",
                            );
                        }
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
                if gwt_git::pr_status::fetch_pr_merge_commit_sha(repo_path, pr).is_some() {
                    let reviewed = record.reviewed_sha.clone().unwrap_or_default();
                    let merged_head =
                        gwt_git::pr_status::fetch_pr_head_sha(repo_path, pr).unwrap_or_default();
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
    let git_root = gwt_git::worktree::main_worktree_root(repo_path)
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let output = gwt_core::process::hidden_command("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(&git_root)
        .output()
        .map_err(|error| GitHubRemoteResolutionError::CommandSpawnFailed(error.to_string()))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    github_remote_owner_and_repo_from_get_url_output(
        output.status.success(),
        output.status.code(),
        &stdout,
        &stderr,
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
        assert_eq!(live.issues, vec![live_issue]);

        let empty_live = resolve_loaded_issue_monitor_candidates(
            Ok(Vec::new()),
            [Ok(vec![cached_issue.clone()])],
        )
        .expect("empty live result still authoritative");
        assert_eq!(empty_live.source, IssueMonitorCandidateSource::Live);
        assert!(empty_live.issues.is_empty());

        let cache = resolve_loaded_issue_monitor_candidates(
            Err("gh unavailable".to_string()),
            [Ok(Vec::new()), Ok(vec![cached_issue.clone()])],
        )
        .expect("cache fallback");
        assert_eq!(cache.source, IssueMonitorCandidateSource::Cache);
        assert_eq!(cache.issues, vec![cached_issue]);

        let error = resolve_loaded_issue_monitor_candidates(
            Err("gh unavailable".to_string()),
            [Ok(Vec::new()), Err("cache corrupt".to_string())],
        )
        .expect_err("no usable cache preserves live error");
        assert_eq!(error, "gh unavailable");
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
    fn parse_default_base_branch_strips_origin_prefix_and_fails_closed() {
        assert_eq!(parse_default_base_branch("origin/main\n"), "main");
        assert_eq!(parse_default_base_branch("origin/develop"), "develop");
        // A bare name with no origin/ prefix is taken as-is.
        assert_eq!(parse_default_base_branch("trunk"), "trunk");
        // Empty / unresolved ⇒ fail-closed to main.
        assert_eq!(parse_default_base_branch(""), "main");
        assert_eq!(parse_default_base_branch("origin/"), "main");
    }
}
