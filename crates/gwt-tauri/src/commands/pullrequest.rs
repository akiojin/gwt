//! Pull Request status commands (gwt-spec issue, gwt-spec issue)

use crate::commands::project::resolve_repo_path_for_project_root;
use chrono::{DateTime, Utc};
use gwt_core::git::gh_cli::{run_gh_output_with_repair, run_gh_output_with_timeout_and_repair};
use gwt_core::git::graphql;
use gwt_core::git::{
    is_gh_cli_authenticated, is_gh_cli_available, Branch, PrCache, PrListItem, PrStatusInfo,
    Remote, ReviewComment, ReviewInfo, WorkflowRunInfo,
};
use gwt_core::StructuredError;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
use tauri::Emitter;
use tracing::warn;

/// gh CLI availability and authentication status
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GhCliStatusInfo {
    pub available: bool,
    pub authenticated: bool,
}

/// Response for fetch_pr_status (T009)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatusResponse {
    pub statuses: HashMap<String, Option<PrStatusLiteSummary>>,
    pub gh_status: GhCliStatusInfo,
    pub repo_key: Option<String>,
}

/// Lightweight PR status summary for Sidebar polling.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatusLiteSummary {
    pub number: u64,
    pub state: String,
    pub url: String,
    pub mergeable: String,
    pub merge_state_status: Option<String>,
    pub merge_ui_state: String,
    pub non_required_checks_warning: bool,
    pub author: String,
    pub base_branch: String,
    pub head_branch: String,
    pub check_suites: Vec<WorkflowRunSummary>,
    pub retrying: bool,
}

/// Serializable workflow run info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowRunSummary {
    pub workflow_name: String,
    pub run_id: u64,
    pub status: String,
    pub conclusion: Option<String>,
    pub is_required: Option<bool>,
}

/// Serializable review info for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewSummary {
    pub reviewer: String,
    pub state: String,
}

/// Serializable review comment for the frontend
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewCommentSummary {
    pub author: String,
    pub body: String,
    pub file_path: Option<String>,
    pub line: Option<u64>,
    pub code_snippet: Option<String>,
    pub created_at: String,
}

/// Response for fetch_pr_detail (T010)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrDetailResponse {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub mergeable: String,
    pub merge_state_status: Option<String>,
    pub merge_ui_state: String,
    pub non_required_checks_warning: bool,
    pub author: String,
    pub base_branch: String,
    pub head_branch: String,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub milestone: Option<String>,
    pub linked_issues: Vec<u64>,
    pub check_suites: Vec<WorkflowRunSummary>,
    pub reviews: Vec<ReviewSummary>,
    pub review_comments: Vec<ReviewCommentSummary>,
    pub changed_files_count: u64,
    pub additions: u64,
    pub deletions: u64,
}

/// Latest PR reference for a branch.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchPrReference {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: Option<String>,
}

/// Preflight status for PR creation on a branch.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct BranchPrPreflightResponse {
    pub base_branch: String,
    pub ahead_by: usize,
    pub behind_by: usize,
    pub status: String,
    pub blocking_reason: Option<String>,
}

#[derive(Debug, Clone)]
struct LatestBranchPrCacheEntry {
    value: Option<BranchPrReference>,
    fetched_at: Instant,
}

const LATEST_BRANCH_PR_CACHE_TTL: Duration = Duration::from_secs(30);
const PR_STATUS_CACHE_TTL: Duration = Duration::from_secs(30);
const PR_STATUS_RATE_LIMIT_BACKOFF: Duration = Duration::from_secs(60);
const RETRY_MAX_ATTEMPTS: u8 = 5;
const RETRY_INITIAL_INTERVAL: Duration = Duration::from_secs(2);
const PR_UPDATE_BRANCH_TIMEOUT: Duration = Duration::from_secs(8);
const PR_MERGE_TIMEOUT: Duration = Duration::from_secs(15);
const FETCH_PR_STATUS_WARN_THRESHOLD: Duration = Duration::from_millis(1000);
const DEFAULT_PR_BASE_BRANCH: &str = "develop";
const PR_BASE_BRANCH_FALLBACKS: [&str; 3] = [DEFAULT_PR_BASE_BRANCH, "main", "master"];

/// Per-PR retry state for UNKNOWN merge status resolution.
#[derive(Debug, Clone)]
struct PrRetryState {
    retrying: bool,
    retry_count: u8,
}

impl PrRetryState {
    fn new() -> Self {
        Self {
            retrying: true,
            retry_count: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
struct RepoPrStatusCacheEntry {
    statuses_by_head_branch: HashMap<String, PrStatusLiteSummary>,
    fetched_at: Option<Instant>,
    cooldown_until: Option<Instant>,
    /// Per-branch retry state for UNKNOWN merge status.
    retry_states: HashMap<String, PrRetryState>,
}

#[derive(Debug, Default)]
struct PrStatusCommandCache {
    repos: HashMap<String, RepoPrStatusCacheEntry>,
}

fn latest_branch_pr_cache() -> &'static Mutex<HashMap<String, LatestBranchPrCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, LatestBranchPrCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn read_latest_branch_pr_cache(cache_key: &str) -> Option<Option<BranchPrReference>> {
    let cache = latest_branch_pr_cache();
    let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
    let entry = guard.get(cache_key)?;
    if entry.fetched_at.elapsed() < LATEST_BRANCH_PR_CACHE_TTL {
        return Some(entry.value.clone());
    }
    guard.remove(cache_key);
    None
}

fn write_latest_branch_pr_cache(cache_key: String, value: Option<BranchPrReference>) {
    let cache = latest_branch_pr_cache();
    let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
    guard.insert(
        cache_key,
        LatestBranchPrCacheEntry {
            value,
            fetched_at: Instant::now(),
        },
    );
}

fn ordered_pr_base_branch_candidates() -> impl Iterator<Item = &'static str> {
    PR_BASE_BRANCH_FALLBACKS.into_iter()
}

fn remote_name_from_ref<'a>(reference: &'a str, remotes: &'a [Remote]) -> Option<&'a str> {
    let normalized = reference
        .trim()
        .strip_prefix("remotes/")
        .unwrap_or(reference);
    let (remote, branch) = normalized.split_once('/')?;
    if branch.is_empty() {
        return None;
    }
    remotes
        .iter()
        .any(|candidate| candidate.name == remote)
        .then_some(remote)
}

fn preferred_preflight_remote_names(remotes: &[Remote]) -> Vec<&str> {
    let mut names: Vec<&str> = remotes.iter().map(|remote| remote.name.as_str()).collect();
    names.sort_by_key(|name| if *name == "origin" { 0 } else { 1 });
    names.dedup();
    names
}

fn resolve_branch_ref_for_preflight(
    repo_path: &Path,
    branch_name: &str,
    remotes: &[Remote],
) -> Option<String> {
    let trimmed = branch_name.trim();
    if trimmed.is_empty() {
        return None;
    }

    if let Some(remote) = remote_name_from_ref(trimmed, remotes) {
        let remote_branch = trimmed
            .trim_start_matches("remotes/")
            .strip_prefix(&format!("{remote}/"))
            .unwrap_or(trimmed);
        if Branch::remote_exists(repo_path, remote, remote_branch).ok()? {
            return Some(format!("{remote}/{remote_branch}"));
        }
    }

    for remote in preferred_preflight_remote_names(remotes) {
        if Branch::remote_exists(repo_path, remote, trimmed).ok()? {
            return Some(format!("{remote}/{trimmed}"));
        }
    }

    if Branch::exists(repo_path, trimmed).ok()? {
        return Some(trimmed.to_string());
    }

    None
}

fn resolve_pr_base_branch(
    repo_path: &Path,
    branch: &str,
    remotes: &[Remote],
    base_branch: Option<String>,
) -> String {
    let explicit = base_branch
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if let Some(value) = explicit {
        return resolve_branch_ref_for_preflight(repo_path, &value, remotes).unwrap_or(value);
    }

    if let Some(pr_base_branch) = PrCache::fetch_latest_for_branch(repo_path, branch)
        .and_then(|pr| pr.base_branch)
        .and_then(|value| resolve_branch_ref_for_preflight(repo_path, &value, remotes))
    {
        return pr_base_branch;
    }

    ordered_pr_base_branch_candidates()
        .find_map(|candidate| resolve_branch_ref_for_preflight(repo_path, candidate, remotes))
        .unwrap_or_else(|| DEFAULT_PR_BASE_BRANCH.to_string())
}

fn refresh_pr_preflight_remote_refs(
    repo_path: &Path,
    base_ref: &str,
    remotes: &[Remote],
) -> Result<(), StructuredError> {
    let remote_name = remote_name_from_ref(base_ref, remotes)
        .map(str::to_string)
        .or_else(|| {
            remotes
                .iter()
                .find(|remote| remote.name == "origin")
                .map(|remote| remote.name.clone())
        })
        .or_else(|| remotes.first().map(|remote| remote.name.clone()));

    let Some(remote_name) = remote_name else {
        return Ok(());
    };

    Remote::fetch(repo_path, &remote_name, false)
        .map_err(|e| StructuredError::internal(&e.to_string(), "fetch_branch_pr_preflight"))?;
    Ok(())
}

fn to_branch_pr_preflight(
    base_branch: String,
    ahead_by: usize,
    behind_by: usize,
) -> BranchPrPreflightResponse {
    let (status, blocking_reason) = match (ahead_by, behind_by) {
        (0, 0) => ("up_to_date".to_string(), None),
        (_, 0) => ("ahead".to_string(), None),
        (0, _) => (
            "behind".to_string(),
            Some("Branch update required before creating a PR.".to_string()),
        ),
        (_, _) => (
            "diverged".to_string(),
            Some("Branch has diverged from base. Sync it before creating a PR.".to_string()),
        ),
    };

    BranchPrPreflightResponse {
        base_branch,
        ahead_by,
        behind_by,
        status,
        blocking_reason,
    }
}

fn pr_status_cache() -> &'static Mutex<PrStatusCommandCache> {
    static CACHE: OnceLock<Mutex<PrStatusCommandCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(PrStatusCommandCache::default()))
}

fn map_cached_statuses(
    branches: &[String],
    cached: &HashMap<String, PrStatusLiteSummary>,
) -> HashMap<String, Option<PrStatusLiteSummary>> {
    branches
        .iter()
        .map(|branch| (branch.clone(), cached.get(branch).cloned()))
        .collect()
}

fn apply_retry_state_overrides(
    statuses: &mut HashMap<String, Option<PrStatusLiteSummary>>,
    retry_states: &HashMap<String, PrRetryState>,
) {
    for summary in statuses.values_mut().flatten() {
        if let Some(retry_state) = retry_states.get(&summary.head_branch) {
            summary.retrying = retry_state.retrying;
            if retry_state.retrying {
                summary.merge_ui_state = "checking".to_string();
            }
        }
    }
}

fn parse_reset_at_to_instant(reset_at: &str) -> Option<Instant> {
    let parsed = DateTime::parse_from_rfc3339(reset_at).ok()?;
    let reset_utc = parsed.with_timezone(&Utc);
    let now = Utc::now();
    if reset_utc <= now {
        return None;
    }
    let delta = reset_utc - now;
    let seconds = u64::try_from(delta.num_seconds()).ok()?;
    Some(Instant::now() + Duration::from_secs(seconds))
}

fn rate_limit_cooldown_until(reset_at: Option<&str>) -> Instant {
    reset_at
        .and_then(parse_reset_at_to_instant)
        .unwrap_or_else(|| Instant::now() + PR_STATUS_RATE_LIMIT_BACKOFF)
}

fn select_repo_merge_method(
    allow_merge_commit: bool,
    allow_squash_merge: bool,
    allow_rebase_merge: bool,
) -> Option<&'static str> {
    if allow_merge_commit {
        Some("merge")
    } else if allow_squash_merge {
        Some("squash")
    } else if allow_rebase_merge {
        Some("rebase")
    } else {
        None
    }
}

fn resolve_repo_merge_method(owner: &str, repo: &str, repo_path: &Path) -> Result<String, String> {
    let output = run_gh_output_with_repair(repo_path, ["api", &format!("/repos/{owner}/{repo}")])
        .map_err(|e| format!("Failed to fetch repository settings: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr.trim();
        if detail.is_empty() {
            return Err("Failed to fetch repository settings".to_string());
        }
        return Err(format!("Failed to fetch repository settings: {detail}"));
    }

    let repo_info: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse repository settings: {}", e))?;

    let allow_merge_commit = repo_info
        .get("allow_merge_commit")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let allow_squash_merge = repo_info
        .get("allow_squash_merge")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let allow_rebase_merge = repo_info
        .get("allow_rebase_merge")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let merge_method =
        select_repo_merge_method(allow_merge_commit, allow_squash_merge, allow_rebase_merge)
            .ok_or_else(|| "No merge methods are enabled for this repository".to_string())?;

    Ok(merge_method.to_string())
}

fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let trimmed = branch.trim();
    let Some((first, rest)) = trimmed.split_once('/') else {
        return trimmed;
    };
    if first == "origin" || remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    trimmed
}

/// Returns true if the PR status has UNKNOWN merge-related fields.
fn has_unknown_mergeable(summary: &PrStatusLiteSummary) -> bool {
    summary.mergeable == "UNKNOWN"
}

fn has_unknown_merge_state_status(summary: &PrStatusLiteSummary) -> bool {
    matches!(summary.merge_state_status.as_deref(), Some("UNKNOWN"))
}

fn has_known_merge_state_status(summary: &PrStatusLiteSummary) -> bool {
    summary
        .merge_state_status
        .as_deref()
        .map(|s| s != "UNKNOWN")
        .unwrap_or(false)
}

fn has_unknown_merge_status(summary: &PrStatusLiteSummary) -> bool {
    has_unknown_mergeable(summary) || has_unknown_merge_state_status(summary)
}

/// Restore only UNKNOWN merge fields from cache while preserving newly-known fields.
fn restore_unknown_merge_fields_from_cache(
    new_summary: &mut PrStatusLiteSummary,
    cached: &PrStatusLiteSummary,
) {
    if has_unknown_mergeable(new_summary) && !has_unknown_mergeable(cached) {
        new_summary.mergeable = cached.mergeable.clone();
    }
    if has_unknown_merge_state_status(new_summary) && has_known_merge_state_status(cached) {
        new_summary.merge_state_status = cached.merge_state_status.clone();
    }
}

fn collect_unknown_branches(
    statuses_by_head_branch: &HashMap<String, PrStatusLiteSummary>,
) -> Vec<String> {
    let mut unknown_branches = statuses_by_head_branch
        .iter()
        .filter(|(_, summary)| has_unknown_merge_status(summary))
        .map(|(branch, _)| branch.clone())
        .collect::<Vec<_>>();
    unknown_branches.sort();
    unknown_branches
}

/// Tauri event payload emitted when a background retry resolves UNKNOWN status.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrStatusUpdatedEvent {
    pub repo_key: String,
    pub branch: String,
    pub status: PrStatusLiteSummary,
}

/// Compute exponential backoff interval for the given attempt (0-indexed).
fn retry_backoff(attempt: u8) -> Duration {
    RETRY_INITIAL_INTERVAL * 2u32.pow(attempt as u32)
}

/// Spawn a background retry task for branches with UNKNOWN merge status.
///
/// This checks if a retry is already in progress and skips if so (FR-008).
/// On resolution, updates cache and emits a Tauri event (FR-004).
fn spawn_unknown_retry(
    repo_key: String,
    repo_path: PathBuf,
    unknown_branches: Vec<String>,
    app_handle: tauri::AppHandle<tauri::Wry>,
) {
    // Filter to branches not already retrying
    let branches_to_retry: Vec<String> = {
        let cache = pr_status_cache();
        let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = guard.repos.entry(repo_key.clone()).or_default();

        let mut to_retry = Vec::new();
        for branch in &unknown_branches {
            let retry_state = entry.retry_states.get(branch);
            if retry_state.map(|s| s.retrying).unwrap_or(false) {
                continue; // Already retrying, skip
            }
            entry
                .retry_states
                .insert(branch.clone(), PrRetryState::new());
            to_retry.push(branch.clone());
        }
        to_retry
    };

    if branches_to_retry.is_empty() {
        return;
    }

    thread::spawn(move || {
        for attempt in 0..RETRY_MAX_ATTEMPTS {
            let delay = retry_backoff(attempt);
            thread::sleep(delay);

            // Check if we're in cooldown
            {
                let cache = pr_status_cache();
                let guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(entry) = guard.repos.get(&repo_key) {
                    if entry
                        .cooldown_until
                        .map(|until| Instant::now() < until)
                        .unwrap_or(false)
                    {
                        // In cooldown, skip this attempt
                        continue;
                    }
                }
            }

            // Find branches still needing retry
            let still_unknown: Vec<String> = {
                let cache = pr_status_cache();
                let guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(entry) = guard.repos.get(&repo_key) {
                    branches_to_retry
                        .iter()
                        .filter(|b| {
                            entry
                                .retry_states
                                .get(*b)
                                .map(|s| s.retrying)
                                .unwrap_or(false)
                        })
                        .cloned()
                        .collect()
                } else {
                    vec![]
                }
            };

            if still_unknown.is_empty() {
                break;
            }

            // Re-fetch using existing query for unknown branches only (FR-003)
            let fetch_result = graphql::fetch_pr_statuses_with_meta(&repo_path, &still_unknown);

            match fetch_result {
                Ok(result) => {
                    let cache = pr_status_cache();
                    let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
                    let entry = guard.repos.entry(repo_key.clone()).or_default();

                    for (branch, info) in &result.by_head_branch {
                        let summary = to_pr_status_summary(info);
                        if !has_unknown_merge_status(&summary) {
                            // Resolved! Update cache and clear retry state
                            let mut resolved = summary;
                            resolved.retrying = false;
                            entry
                                .statuses_by_head_branch
                                .insert(branch.clone(), resolved.clone());
                            entry.retry_states.remove(branch);

                            // Emit event to frontend (FR-004)
                            let _ = app_handle.emit(
                                "pr-status-updated",
                                PrStatusUpdatedEvent {
                                    repo_key: repo_key.clone(),
                                    branch: branch.clone(),
                                    status: resolved,
                                },
                            );
                        }
                    }

                    // Update retry counts
                    for branch in &still_unknown {
                        if let Some(state) = entry.retry_states.get_mut(branch) {
                            state.retry_count = attempt + 1;
                        }
                    }
                }
                Err(error) => {
                    if graphql::is_rate_limit_error(&error) {
                        let cache = pr_status_cache();
                        let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
                        let entry = guard.repos.entry(repo_key.clone()).or_default();
                        entry.cooldown_until = Some(Instant::now() + PR_STATUS_RATE_LIMIT_BACKOFF);
                    }
                    // Continue to next attempt
                }
            }

            // Check if all resolved
            {
                let cache = pr_status_cache();
                let guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(entry) = guard.repos.get(&repo_key) {
                    let any_still_retrying = branches_to_retry.iter().any(|b| {
                        entry
                            .retry_states
                            .get(b)
                            .map(|s| s.retrying)
                            .unwrap_or(false)
                    });
                    if !any_still_retrying {
                        break;
                    }
                }
            }
        }

        // Clean up: clear retry states for all branches we were handling (FR-011)
        {
            let cache = pr_status_cache();
            let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
            if let Some(entry) = guard.repos.get_mut(&repo_key) {
                for branch in &branches_to_retry {
                    entry.retry_states.remove(branch);
                }
            }
        }
    });
}

fn to_workflow_run_summary(info: &WorkflowRunInfo) -> WorkflowRunSummary {
    WorkflowRunSummary {
        workflow_name: info.workflow_name.clone(),
        run_id: info.run_id,
        status: info.status.clone(),
        conclusion: info.conclusion.clone(),
        is_required: info.is_required,
    }
}

fn to_review_summary(info: &ReviewInfo) -> ReviewSummary {
    ReviewSummary {
        reviewer: info.reviewer.clone(),
        state: info.state.clone(),
    }
}

fn to_review_comment_summary(comment: &ReviewComment) -> ReviewCommentSummary {
    ReviewCommentSummary {
        author: comment.author.clone(),
        body: comment.body.clone(),
        file_path: comment.file_path.clone(),
        line: comment.line,
        code_snippet: comment.code_snippet.clone(),
        created_at: comment.created_at.clone(),
    }
}

fn is_failure_conclusion(conclusion: Option<&str>) -> bool {
    matches!(
        conclusion,
        Some("failure" | "cancelled" | "timed_out" | "action_required" | "startup_failure")
    )
}

fn has_required_check_failure(check_suites: &[WorkflowRunInfo]) -> bool {
    check_suites.iter().any(|run| {
        run.is_required == Some(true) && is_failure_conclusion(run.conclusion.as_deref())
    })
}

fn has_non_required_check_failure(check_suites: &[WorkflowRunInfo]) -> bool {
    check_suites.iter().any(|run| {
        run.is_required == Some(false) && is_failure_conclusion(run.conclusion.as_deref())
    })
}

fn compute_non_required_checks_warning(check_suites: &[WorkflowRunInfo]) -> bool {
    has_non_required_check_failure(check_suites) && !has_required_check_failure(check_suites)
}

fn has_changes_requested(reviews: &[ReviewInfo]) -> bool {
    let mut latest_state_by_reviewer: HashMap<&str, &str> = HashMap::new();
    for review in reviews {
        // GraphQL reviews(last: N) returns chronological order within the returned page.
        // Overwriting keeps the latest state for each reviewer in the sampled window.
        latest_state_by_reviewer.insert(review.reviewer.as_str(), review.state.as_str());
    }
    latest_state_by_reviewer
        .values()
        .any(|state| *state == "CHANGES_REQUESTED")
}

fn is_unknown_merge_fields(mergeable: &str, merge_state_status: Option<&str>) -> bool {
    mergeable == "UNKNOWN" || matches!(merge_state_status, Some("UNKNOWN"))
}

fn compute_merge_ui_state(
    state: &str,
    mergeable: &str,
    merge_state_status: Option<&str>,
    retrying: bool,
    check_suites: &[WorkflowRunInfo],
    reviews: &[ReviewInfo],
) -> &'static str {
    if state == "MERGED" {
        return "merged";
    }
    if state == "CLOSED" {
        return "closed";
    }
    if retrying {
        return "checking";
    }
    if merge_state_status == Some("BLOCKED")
        || has_required_check_failure(check_suites)
        || has_changes_requested(reviews)
    {
        return "blocked";
    }
    if is_unknown_merge_fields(mergeable, merge_state_status) {
        return "checking";
    }
    if mergeable == "CONFLICTING" {
        return "conflicting";
    }
    "mergeable"
}

fn to_pr_status_summary(info: &PrStatusInfo) -> PrStatusLiteSummary {
    let merge_ui_state = compute_merge_ui_state(
        &info.state,
        &info.mergeable,
        info.merge_state_status.as_deref(),
        false,
        &info.check_suites,
        &info.reviews,
    )
    .to_string();
    let non_required_checks_warning = compute_non_required_checks_warning(&info.check_suites);
    PrStatusLiteSummary {
        number: info.number,
        state: info.state.clone(),
        url: info.url.clone(),
        mergeable: info.mergeable.clone(),
        merge_state_status: info.merge_state_status.clone(),
        merge_ui_state,
        non_required_checks_warning,
        author: info.author.clone(),
        base_branch: info.base_branch.clone(),
        head_branch: info.head_branch.clone(),
        check_suites: info
            .check_suites
            .iter()
            .map(to_workflow_run_summary)
            .collect(),
        retrying: false,
    }
}

fn to_pr_detail_response(info: &PrStatusInfo) -> PrDetailResponse {
    let merge_ui_state = compute_merge_ui_state(
        &info.state,
        &info.mergeable,
        info.merge_state_status.as_deref(),
        false,
        &info.check_suites,
        &info.reviews,
    )
    .to_string();
    let non_required_checks_warning = compute_non_required_checks_warning(&info.check_suites);
    PrDetailResponse {
        number: info.number,
        title: info.title.clone(),
        state: info.state.clone(),
        url: info.url.clone(),
        mergeable: info.mergeable.clone(),
        merge_state_status: info.merge_state_status.clone(),
        merge_ui_state,
        non_required_checks_warning,
        author: info.author.clone(),
        base_branch: info.base_branch.clone(),
        head_branch: info.head_branch.clone(),
        labels: info.labels.clone(),
        assignees: info.assignees.clone(),
        milestone: info.milestone.clone(),
        linked_issues: info.linked_issues.clone(),
        check_suites: info
            .check_suites
            .iter()
            .map(to_workflow_run_summary)
            .collect(),
        reviews: info.reviews.iter().map(to_review_summary).collect(),
        review_comments: info
            .review_comments
            .iter()
            .map(to_review_comment_summary)
            .collect(),
        changed_files_count: info.changed_files_count,
        additions: info.additions,
        deletions: info.deletions,
    }
}

/// Internal result from fetch_pr_status_impl, carrying retry metadata.
struct FetchPrStatusResult {
    response: PrStatusResponse,
    /// Branches that have UNKNOWN merge status and need retry.
    unknown_branches: Vec<String>,
    /// Resolved repo path for retry.
    repo_path: Option<PathBuf>,
    /// Cache key for the repo.
    repo_key: Option<String>,
}

/// Fetch PR statuses for all given branches via GraphQL (T009)
///
/// Also returns gh CLI availability/authentication status.
fn fetch_pr_status_impl(
    project_path: String,
    branches: Vec<String>,
) -> Result<FetchPrStatusResult, StructuredError> {
    let available = is_gh_cli_available();
    let authenticated = if available {
        is_gh_cli_authenticated()
    } else {
        false
    };
    let gh_status = GhCliStatusInfo {
        available,
        authenticated,
    };

    if !available || !authenticated {
        // Return empty statuses with gh_status indicating the problem
        let statuses = branches.into_iter().map(|branch| (branch, None)).collect();
        return Ok(FetchPrStatusResult {
            response: PrStatusResponse {
                statuses,
                gh_status,
                repo_key: None,
            },
            unknown_branches: vec![],
            repo_path: None,
            repo_key: None,
        });
    }

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_status"))?;
    let repo_key = repo_path.to_string_lossy().to_string();
    let now = Instant::now();

    {
        let cache = pr_status_cache();
        let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = guard.repos.entry(repo_key.clone()).or_default();

        let cache_is_fresh = entry
            .fetched_at
            .map(|fetched_at| now.saturating_duration_since(fetched_at) < PR_STATUS_CACHE_TTL)
            .unwrap_or(false);
        let in_cooldown = entry
            .cooldown_until
            .map(|until| now < until)
            .unwrap_or(false);

        if cache_is_fresh || in_cooldown {
            // Mark retrying PRs in the cache response
            let mut statuses = map_cached_statuses(&branches, &entry.statuses_by_head_branch);
            apply_retry_state_overrides(&mut statuses, &entry.retry_states);
            return Ok(FetchPrStatusResult {
                response: PrStatusResponse {
                    statuses,
                    gh_status,
                    repo_key: Some(repo_key.clone()),
                },
                unknown_branches: vec![],
                repo_path: None,
                repo_key: None,
            });
        }
    }

    let fetch_result = graphql::fetch_pr_statuses_with_meta(&repo_path, &branches);

    let (statuses_by_head_branch, cooldown_until) = match fetch_result {
        Ok(result) => {
            let statuses_by_head_branch = result
                .by_head_branch
                .iter()
                .map(|(branch, info)| (branch.clone(), to_pr_status_summary(info)))
                .collect::<HashMap<_, _>>();
            let cooldown_until = match result.rate_limit.remaining {
                Some(0) => Some(rate_limit_cooldown_until(
                    result.rate_limit.reset_at.as_deref(),
                )),
                _ => None,
            };
            (statuses_by_head_branch, cooldown_until)
        }
        Err(error) => {
            let cache = pr_status_cache();
            let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
            let entry = guard.repos.entry(repo_key.clone()).or_default();
            if graphql::is_rate_limit_error(&error) {
                entry.cooldown_until = Some(Instant::now() + PR_STATUS_RATE_LIMIT_BACKOFF);
            }
            // Silent degrade: use stale cache if available, otherwise no statuses.
            let mut statuses = map_cached_statuses(&branches, &entry.statuses_by_head_branch);
            apply_retry_state_overrides(&mut statuses, &entry.retry_states);
            return Ok(FetchPrStatusResult {
                response: PrStatusResponse {
                    statuses,
                    gh_status,
                    repo_key: Some(repo_key),
                },
                unknown_branches: vec![],
                repo_path: None,
                repo_key: None,
            });
        }
    };

    // Detect UNKNOWN branches from the raw fetch result (before cache restoration).
    let unknown_branches = collect_unknown_branches(&statuses_by_head_branch);

    // Write cache with UNKNOWN protection (FR-005):
    // If the new result has UNKNOWN merge fields but the cache already has
    // a known value, preserve the cached merge fields instead of regressing.
    let final_statuses = {
        let cache = pr_status_cache();
        let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = guard.repos.entry(repo_key.clone()).or_default();

        let mut merged = statuses_by_head_branch.clone();
        for (branch, new_summary) in &mut merged {
            if let Some(cached) = entry.statuses_by_head_branch.get(branch) {
                restore_unknown_merge_fields_from_cache(new_summary, cached);
            }
        }

        entry.statuses_by_head_branch = merged.clone();
        entry.fetched_at = Some(now);
        entry.cooldown_until = cooldown_until;
        merged
    };

    let mut statuses = map_cached_statuses(&branches, &final_statuses);

    // Set retrying flag on UNKNOWN PR statuses
    if !unknown_branches.is_empty() {
        for summary in statuses.values_mut().flatten() {
            if unknown_branches.contains(&summary.head_branch) {
                summary.retrying = true;
                summary.merge_ui_state = "checking".to_string();
            }
        }
    }

    Ok(FetchPrStatusResult {
        response: PrStatusResponse {
            statuses,
            gh_status,
            repo_key: Some(repo_key.clone()),
        },
        unknown_branches,
        repo_path: Some(repo_path),
        repo_key: Some(repo_key),
    })
}

#[tauri::command]
pub async fn fetch_pr_status(
    app: tauri::AppHandle<tauri::Wry>,
    project_path: String,
    branches: Vec<String>,
) -> Result<PrStatusResponse, StructuredError> {
    let started = Instant::now();
    let inner =
        tauri::async_runtime::spawn_blocking(move || fetch_pr_status_impl(project_path, branches))
            .await
            .map_err(|e| {
                StructuredError::internal(&format!("Task join failed: {e}"), "fetch_pr_status")
            })??;
    let elapsed = started.elapsed();
    if elapsed > FETCH_PR_STATUS_WARN_THRESHOLD {
        warn!(
            category = "pullrequest",
            elapsed_ms = elapsed.as_millis(),
            "fetch_pr_status took longer than expected"
        );
    }

    // Spawn background retry for UNKNOWN branches (FR-001, FR-002, T004/T005)
    if !inner.unknown_branches.is_empty() {
        if let (Some(repo_path), Some(repo_key)) = (inner.repo_path, inner.repo_key) {
            spawn_unknown_retry(repo_key, repo_path, inner.unknown_branches, app);
        }
    }

    Ok(inner.response)
}

/// Fetch detailed PR information for a single PR (T010)
fn fetch_pr_detail_impl(
    project_path: String,
    pr_number: u64,
) -> Result<PrDetailResponse, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_detail"))?;

    let info = graphql::fetch_pr_detail(&repo_path, pr_number)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_detail"))?;
    Ok(to_pr_detail_response(&info))
}

#[tauri::command]
pub async fn fetch_pr_detail(
    project_path: String,
    pr_number: u64,
) -> Result<PrDetailResponse, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || fetch_pr_detail_impl(project_path, pr_number))
        .await
        .map_err(|e| {
            StructuredError::internal(&format!("Task join failed: {e}"), "fetch_pr_detail")
        })?
}

/// Fetch latest branch PR: open PR first, otherwise latest closed/merged.
fn fetch_latest_branch_pr_impl(
    project_path: String,
    branch: String,
) -> Result<Option<BranchPrReference>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_latest_branch_pr"))?;
    let remotes = Remote::list(&repo_path).unwrap_or_default();
    let normalized = strip_known_remote_prefix(&branch, &remotes);
    if normalized.is_empty() {
        return Ok(None);
    }

    let cache_key = format!("{}::{}", repo_path.to_string_lossy(), normalized);
    if let Some(cached) = read_latest_branch_pr_cache(&cache_key) {
        return Ok(cached);
    }

    let latest = PrCache::fetch_latest_for_branch(&repo_path, normalized);
    let result = latest.map(|pr| BranchPrReference {
        number: pr.number,
        title: pr.title,
        state: pr.state,
        url: pr.url,
    });
    write_latest_branch_pr_cache(cache_key, result.clone());

    Ok(result)
}

#[tauri::command]
pub async fn fetch_latest_branch_pr(
    project_path: String,
    branch: String,
) -> Result<Option<BranchPrReference>, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || fetch_latest_branch_pr_impl(project_path, branch))
        .await
        .map_err(|e| {
            StructuredError::internal(&format!("Task join failed: {e}"), "fetch_latest_branch_pr")
        })?
}

/// Fetch CI run log for a specific check run/job ID (T011)
fn fetch_ci_log_impl(project_path: String, run_id: u64) -> Result<String, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_ci_log"))?;

    let output = gwt_core::git::graphql::gh_run_view_log(&repo_path, run_id)
        .map_err(|e| StructuredError::internal(&e, "fetch_ci_log"))?;
    Ok(output)
}

#[tauri::command]
pub async fn fetch_ci_log(project_path: String, run_id: u64) -> Result<String, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || fetch_ci_log_impl(project_path, run_id))
        .await
        .map_err(|e| StructuredError::internal(&format!("Task join failed: {e}"), "fetch_ci_log"))?
}

/// Update a PR branch with the latest base branch changes (gwt-spec issue T008)
fn update_pr_branch_impl(project_path: String, pr_number: u64) -> Result<String, String> {
    use gwt_core::git::resolve_repo_slug;

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let slug = resolve_repo_slug(&repo_path)
        .ok_or_else(|| "Failed to resolve repository slug".to_string())?;
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid repo slug: {}", slug));
    }
    let (owner, repo) = (parts[0], parts[1]);

    let output = run_gh_output_with_timeout_and_repair(
        &repo_path,
        [
            "api",
            "-X",
            "PUT",
            &format!("/repos/{owner}/{repo}/pulls/{pr_number}/update-branch"),
        ],
        PR_UPDATE_BRANCH_TIMEOUT,
    )
    .map_err(|e| format!("Failed to execute gh api: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let detail = stderr.trim();
        if detail.is_empty() {
            return Err("Failed to update PR branch".to_string());
        }
        return Err(format!("Failed to update PR branch: {detail}"));
    }

    Ok("Branch updated successfully".to_string())
}

#[tauri::command]
pub async fn update_pr_branch(project_path: String, pr_number: u64) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || update_pr_branch_impl(project_path, pr_number))
        .await
        .map_err(|e| format!("Task join failed: {e}"))?
}

fn fetch_branch_pr_preflight_impl(
    project_path: String,
    branch: String,
    base_branch: Option<String>,
) -> Result<BranchPrPreflightResponse, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_branch_pr_preflight"))?;
    let remotes = Remote::list(&repo_path).unwrap_or_default();

    let normalized_branch = strip_known_remote_prefix(branch.trim(), &remotes);
    if normalized_branch.is_empty() {
        return Err(StructuredError::internal(
            "Branch name is required",
            "fetch_branch_pr_preflight",
        ));
    }

    let base_ref = resolve_pr_base_branch(&repo_path, normalized_branch, &remotes, base_branch);
    refresh_pr_preflight_remote_refs(&repo_path, &base_ref, &remotes)?;
    let (ahead_by, behind_by) =
        Branch::divergence_between(&repo_path, normalized_branch, &base_ref)
            .map_err(|e| StructuredError::internal(&e.to_string(), "fetch_branch_pr_preflight"))?;

    Ok(to_branch_pr_preflight(base_ref, ahead_by, behind_by))
}

#[tauri::command]
pub async fn fetch_branch_pr_preflight(
    project_path: String,
    branch: String,
    base_branch: Option<String>,
) -> Result<BranchPrPreflightResponse, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        fetch_branch_pr_preflight_impl(project_path, branch, base_branch)
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Task join failed: {e}"),
            "fetch_branch_pr_preflight",
        )
    })?
}

/// Merge a pull request via GitHub REST API (gwt-spec issue FR-004)
fn merge_pull_request_impl(project_path: String, pr_number: u64) -> Result<String, String> {
    use gwt_core::git::resolve_repo_slug;

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let slug = resolve_repo_slug(&repo_path)
        .ok_or_else(|| "Failed to resolve repository slug".to_string())?;
    let parts: Vec<&str> = slug.split('/').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid repo slug: {}", slug));
    }
    let (owner, repo) = (parts[0], parts[1]);
    let merge_method = resolve_repo_merge_method(owner, repo, &repo_path)?;

    let output = run_gh_output_with_timeout_and_repair(
        &repo_path,
        [
            "api",
            "-X",
            "PUT",
            &format!("/repos/{owner}/{repo}/pulls/{pr_number}/merge"),
            "-f",
            &format!("merge_method={merge_method}"),
        ],
        PR_MERGE_TIMEOUT,
    )
    .map_err(|e| format!("Failed to execute gh api: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let detail = stderr.trim();
        if detail.is_empty() {
            return Err("Failed to merge PR".to_string());
        }
        return Err(format!("Failed to merge PR: {detail}"));
    }

    Ok("Pull request merged successfully".to_string())
}

#[tauri::command]
pub async fn merge_pull_request(project_path: String, pr_number: u64) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || merge_pull_request_impl(project_path, pr_number))
        .await
        .map_err(|e| format!("Task join failed: {e}"))?
}

// ==========================================================
// PR Dashboard commands (gwt-spec issue)
// ==========================================================

/// Response for fetch_pr_list
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchPrListResponse {
    pub items: Vec<PrListItem>,
    pub gh_status: GhCliStatusInfo,
}

/// Response for fetch_github_user
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubUserResponse {
    pub login: String,
    pub gh_status: GhCliStatusInfo,
}

const GITHUB_USER_CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Debug, Clone)]
struct GitHubUserCacheEntry {
    login: String,
    fetched_at: Instant,
}

fn github_user_cache() -> &'static Mutex<HashMap<String, GitHubUserCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, GitHubUserCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn extract_remote_host(remote_url: &str) -> Option<String> {
    let trimmed = remote_url.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.starts_with("file://") {
        return None;
    }

    if let Some((_, rest)) = trimmed.split_once("://") {
        let rest = rest.rsplit_once('@').map(|(_, host)| host).unwrap_or(rest);
        let host_end = rest
            .find('/')
            .or_else(|| rest.find(':'))
            .unwrap_or(rest.len());
        let host = rest.get(..host_end)?.trim();
        if host.is_empty() {
            return None;
        }
        return Some(host.to_ascii_lowercase());
    }

    let after_at = trimmed
        .split_once('@')
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host_end = after_at
        .find(':')
        .or_else(|| after_at.find('/'))
        .unwrap_or(after_at.len());
    let host = after_at.get(..host_end)?.trim();
    if host.is_empty() {
        return None;
    }
    Some(host.to_ascii_lowercase())
}

fn github_user_cache_key(repo_path: &Path) -> String {
    let host = Remote::default(repo_path)
        .ok()
        .flatten()
        .and_then(|remote| {
            extract_remote_host(&remote.fetch_url).or_else(|| extract_remote_host(&remote.push_url))
        })
        .unwrap_or_else(|| "unknown".to_string());
    format!("{host}::{}", repo_path.to_string_lossy())
}

fn fetch_pr_list_impl(
    project_path: String,
    state: String,
    limit: u32,
) -> Result<FetchPrListResponse, StructuredError> {
    let available = is_gh_cli_available();
    let authenticated = if available {
        is_gh_cli_authenticated()
    } else {
        false
    };
    let gh_status = GhCliStatusInfo {
        available,
        authenticated,
    };

    if !available || !authenticated {
        return Ok(FetchPrListResponse {
            items: vec![],
            gh_status,
        });
    }

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_list"))?;

    let raw_items = gwt_core::git::gh_cli::fetch_pr_list(&repo_path, &state, limit)
        .map_err(|e| StructuredError::internal(&e, "fetch_pr_list"))?;

    let items: Vec<PrListItem> = raw_items
        .into_iter()
        .filter_map(|v| serde_json::from_value(v).ok())
        .collect();

    Ok(FetchPrListResponse { items, gh_status })
}

#[tauri::command]
pub async fn fetch_pr_list(
    project_path: String,
    state: String,
    limit: u32,
) -> Result<FetchPrListResponse, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || fetch_pr_list_impl(project_path, state, limit))
        .await
        .map_err(|e| {
            StructuredError::internal(&format!("Task join failed: {e}"), "fetch_pr_list")
        })?
}

fn fetch_github_user_impl(project_path: String) -> Result<GitHubUserResponse, StructuredError> {
    let available = is_gh_cli_available();
    let authenticated = if available {
        is_gh_cli_authenticated()
    } else {
        false
    };
    let gh_status = GhCliStatusInfo {
        available,
        authenticated,
    };

    if !available || !authenticated {
        return Err(StructuredError::internal(
            "gh CLI is not available or not authenticated",
            "fetch_github_user",
        ));
    }

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_user"))?;
    let cache_key = github_user_cache_key(&repo_path);

    // Check cache
    {
        let cache = github_user_cache();
        let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
        if let Some(entry) = guard.get(&cache_key) {
            if entry.fetched_at.elapsed() < GITHUB_USER_CACHE_TTL {
                return Ok(GitHubUserResponse {
                    login: entry.login.clone(),
                    gh_status,
                });
            }
            guard.remove(&cache_key);
        }
    }

    let login = gwt_core::git::gh_cli::fetch_authenticated_user(&repo_path)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_user"))?;

    // Update cache
    {
        let cache = github_user_cache();
        let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
        guard.insert(
            cache_key,
            GitHubUserCacheEntry {
                login: login.clone(),
                fetched_at: Instant::now(),
            },
        );
    }

    Ok(GitHubUserResponse { login, gh_status })
}

#[tauri::command]
pub async fn fetch_github_user(
    project_path: String,
) -> Result<GitHubUserResponse, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || fetch_github_user_impl(project_path))
        .await
        .map_err(|e| {
            StructuredError::internal(&format!("Task join failed: {e}"), "fetch_github_user")
        })?
}

fn merge_pr_impl(
    project_path: String,
    pr_number: u64,
    method: String,
    delete_branch: bool,
    commit_msg: Option<String>,
) -> Result<String, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    gwt_core::git::gh_cli::merge_pr(
        &repo_path,
        pr_number,
        &method,
        delete_branch,
        commit_msg.as_deref(),
    )
}

#[tauri::command]
pub async fn merge_pr(
    project_path: String,
    pr_number: u64,
    method: String,
    delete_branch: bool,
    commit_msg: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        merge_pr_impl(project_path, pr_number, method, delete_branch, commit_msg)
    })
    .await
    .map_err(|e| format!("Task join failed: {e}"))?
}

fn review_pr_impl(
    project_path: String,
    pr_number: u64,
    action: String,
    body: Option<String>,
) -> Result<String, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    gwt_core::git::gh_cli::review_pr(&repo_path, pr_number, &action, body.as_deref())
}

#[tauri::command]
pub async fn review_pr(
    project_path: String,
    pr_number: u64,
    action: String,
    body: Option<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || {
        review_pr_impl(project_path, pr_number, action, body)
    })
    .await
    .map_err(|e| format!("Task join failed: {e}"))?
}

fn mark_pr_ready_impl(project_path: String, pr_number: u64) -> Result<String, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    gwt_core::git::gh_cli::mark_pr_ready(&repo_path, pr_number)
}

#[tauri::command]
pub async fn mark_pr_ready(project_path: String, pr_number: u64) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || mark_pr_ready_impl(project_path, pr_number))
        .await
        .map_err(|e| format!("Task join failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn run_git(repo_path: &Path, args: &[&str]) {
        let output = gwt_core::process::command("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .unwrap_or_else(|err| panic!("git {:?} failed to start: {err}", args));
        assert!(
            output.status.success(),
            "git {:?} failed: stdout={}, stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn setup_pr_preflight_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        let origin = temp.path().join("origin.git");
        let repo = temp.path().join("repo");
        let updater = temp.path().join("updater");

        std::fs::create_dir_all(&repo).unwrap();
        run_git(temp.path(), &["init", "--bare", origin.to_str().unwrap()]);

        run_git(&repo, &["init"]);
        run_git(&repo, &["config", "user.email", "test@example.com"]);
        run_git(&repo, &["config", "user.name", "Test User"]);
        std::fs::write(repo.join("README.md"), "# Test\n").unwrap();
        run_git(&repo, &["add", "README.md"]);
        run_git(&repo, &["commit", "-m", "initial"]);
        run_git(&repo, &["checkout", "-b", "develop"]);
        run_git(
            &repo,
            &["remote", "add", "origin", origin.to_str().unwrap()],
        );
        run_git(&repo, &["push", "-u", "origin", "develop"]);
        run_git(&repo, &["checkout", "-b", "feature/preflight"]);
        run_git(&repo, &["push", "-u", "origin", "feature/preflight"]);

        run_git(
            temp.path(),
            &["clone", origin.to_str().unwrap(), updater.to_str().unwrap()],
        );
        run_git(&updater, &["config", "user.email", "test@example.com"]);
        run_git(&updater, &["config", "user.name", "Test User"]);
        run_git(&updater, &["checkout", "develop"]);
        std::fs::write(updater.join("develop.txt"), "advanced\n").unwrap();
        run_git(&updater, &["add", "develop.txt"]);
        run_git(&updater, &["commit", "-m", "advance develop"]);
        run_git(&updater, &["push", "origin", "develop"]);

        run_git(&repo, &["checkout", "feature/preflight"]);
        temp
    }

    // ==========================================================
    // T012: GhCliStatusInfo serialization tests
    // ==========================================================

    #[test]
    fn test_gh_cli_status_info_serialization() {
        let status = GhCliStatusInfo {
            available: true,
            authenticated: true,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"available\":true"));
        assert!(json.contains("\"authenticated\":true"));
    }

    #[test]
    fn test_gh_cli_status_info_unavailable() {
        let status = GhCliStatusInfo {
            available: false,
            authenticated: false,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"available\":false"));
        assert!(json.contains("\"authenticated\":false"));
    }

    // ==========================================================
    // T012: PrStatusResponse serialization tests
    // ==========================================================

    #[test]
    fn test_pr_status_response_serialization() {
        let mut statuses = HashMap::new();
        statuses.insert(
            "feature/x".to_string(),
            Some(PrStatusLiteSummary {
                number: 42,
                state: "OPEN".to_string(),
                url: "https://github.com/o/r/pull/42".to_string(),
                mergeable: "MERGEABLE".to_string(),
                merge_state_status: None,
                merge_ui_state: "mergeable".to_string(),
                non_required_checks_warning: false,
                author: "alice".to_string(),
                base_branch: "main".to_string(),
                head_branch: "feature/x".to_string(),
                check_suites: vec![WorkflowRunSummary {
                    workflow_name: "CI".to_string(),
                    run_id: 12345,
                    status: "completed".to_string(),
                    conclusion: Some("success".to_string()),
                    is_required: None,
                }],
                retrying: false,
            }),
        );
        statuses.insert("feature/y".to_string(), None);

        let response = PrStatusResponse {
            statuses,
            gh_status: GhCliStatusInfo {
                available: true,
                authenticated: true,
            },
            repo_key: Some("/tmp/repo.git".to_string()),
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"statuses\""));
        assert!(json.contains("\"ghStatus\""));
        assert!(json.contains("\"repoKey\":\"/tmp/repo.git\""));
        assert!(json.contains("\"available\":true"));
        assert!(json.contains("\"number\":42"));
        assert!(json.contains("\"baseBranch\":\"main\""));
        assert!(json.contains("\"checkSuites\""));
        assert!(json.contains("\"workflowName\":\"CI\""));
        assert!(!json.contains("changedFilesCount"));
    }

    #[test]
    fn test_pr_status_response_empty() {
        let response = PrStatusResponse {
            statuses: HashMap::new(),
            gh_status: GhCliStatusInfo {
                available: false,
                authenticated: false,
            },
            repo_key: None,
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"statuses\":{}"));
        assert!(json.contains("\"ghStatus\""));
        assert!(json.contains("\"repoKey\":null"));
        assert!(json.contains("\"available\":false"));
    }

    // ==========================================================
    // T012: PrDetailResponse serialization tests
    // ==========================================================

    #[test]
    fn test_pr_detail_response_serialization() {
        let response = PrDetailResponse {
            number: 42,
            title: "Detailed PR".to_string(),
            state: "OPEN".to_string(),
            url: "https://github.com/o/r/pull/42".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: None,
            merge_ui_state: "mergeable".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/detail".to_string(),
            labels: vec!["bug".to_string()],
            assignees: vec![],
            milestone: None,
            linked_issues: vec![],
            check_suites: vec![],
            reviews: vec![ReviewSummary {
                reviewer: "bob".to_string(),
                state: "CHANGES_REQUESTED".to_string(),
            }],
            review_comments: vec![ReviewCommentSummary {
                author: "bob".to_string(),
                body: "Fix this line".to_string(),
                file_path: Some("src/main.rs".to_string()),
                line: Some(42),
                code_snippet: None,
                created_at: "2025-01-01T00:00:00Z".to_string(),
            }],
            changed_files_count: 3,
            additions: 50,
            deletions: 10,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"reviewComments\""));
        assert!(json.contains("\"filePath\":\"src/main.rs\""));
        assert!(json.contains("\"createdAt\":\"2025-01-01T00:00:00Z\""));
        assert!(json.contains("\"changedFilesCount\":3"));
    }

    // ==========================================================
    // T012: Conversion function tests
    // ==========================================================

    #[test]
    fn test_to_pr_status_summary() {
        let info = PrStatusInfo {
            number: 1,
            title: "Test".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com".to_string(),
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: None,
            author: "user".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/test".to_string(),
            labels: vec!["label".to_string()],
            assignees: vec!["a".to_string()],
            milestone: Some("m1".to_string()),
            linked_issues: vec![5],
            check_suites: vec![WorkflowRunInfo {
                workflow_name: "CI".to_string(),
                run_id: 100,
                status: "completed".to_string(),
                conclusion: Some("success".to_string()),
                is_required: None,
            }],
            reviews: vec![ReviewInfo {
                reviewer: "r1".to_string(),
                state: "APPROVED".to_string(),
            }],
            review_comments: vec![],
            changed_files_count: 2,
            additions: 10,
            deletions: 3,
        };

        let summary = to_pr_status_summary(&info);
        assert_eq!(summary.number, 1);
        assert_eq!(summary.head_branch, "feature/test");
        assert_eq!(summary.check_suites.len(), 1);
        assert_eq!(summary.check_suites[0].workflow_name, "CI");
        assert_eq!(summary.mergeable, "UNKNOWN");
        assert_eq!(summary.merge_ui_state, "checking");
    }

    #[test]
    fn test_to_pr_detail_response() {
        let info = PrStatusInfo {
            number: 10,
            title: "Detail".to_string(),
            state: "OPEN".to_string(),
            url: "https://example.com/10".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: None,
            author: "user".to_string(),
            base_branch: "main".to_string(),
            head_branch: "fix/bug".to_string(),
            labels: vec![],
            assignees: vec![],
            milestone: None,
            linked_issues: vec![],
            check_suites: vec![],
            reviews: vec![],
            review_comments: vec![ReviewComment {
                author: "reviewer".to_string(),
                body: "Comment".to_string(),
                file_path: Some("file.rs".to_string()),
                line: Some(5),
                code_snippet: None,
                created_at: "2025-01-01T00:00:00Z".to_string(),
            }],
            changed_files_count: 1,
            additions: 5,
            deletions: 0,
        };

        let detail = to_pr_detail_response(&info);
        assert_eq!(detail.number, 10);
        assert_eq!(detail.review_comments.len(), 1);
        assert_eq!(detail.review_comments[0].author, "reviewer");
        assert_eq!(
            detail.review_comments[0].file_path,
            Some("file.rs".to_string())
        );
        assert_eq!(detail.merge_ui_state, "mergeable");
        assert!(!detail.non_required_checks_warning);
    }

    #[test]
    fn test_compute_merge_ui_state_blocked_by_required_check_failure() {
        let checks = vec![WorkflowRunInfo {
            workflow_name: "CI".to_string(),
            run_id: 100,
            status: "completed".to_string(),
            conclusion: Some("failure".to_string()),
            is_required: Some(true),
        }];
        let reviews = vec![];

        let state =
            compute_merge_ui_state("OPEN", "MERGEABLE", Some("CLEAN"), false, &checks, &reviews);
        assert_eq!(state, "blocked");
    }

    #[test]
    fn test_compute_merge_ui_state_checking_for_unknown_fields() {
        let state = compute_merge_ui_state("OPEN", "UNKNOWN", Some("UNKNOWN"), false, &[], &[]);
        assert_eq!(state, "checking");
    }

    #[test]
    fn test_compute_merge_ui_state_prioritizes_blocked_over_unknown() {
        let checks = vec![WorkflowRunInfo {
            workflow_name: "Required CI".to_string(),
            run_id: 20,
            status: "completed".to_string(),
            conclusion: Some("failure".to_string()),
            is_required: Some(true),
        }];
        let state = compute_merge_ui_state("OPEN", "UNKNOWN", Some("UNKNOWN"), false, &checks, &[]);
        assert_eq!(state, "blocked");
    }

    #[test]
    fn test_compute_merge_ui_state_blocked_by_startup_failure() {
        let checks = vec![WorkflowRunInfo {
            workflow_name: "Required CI".to_string(),
            run_id: 21,
            status: "completed".to_string(),
            conclusion: Some("startup_failure".to_string()),
            is_required: Some(true),
        }];
        let state = compute_merge_ui_state("OPEN", "MERGEABLE", Some("CLEAN"), false, &checks, &[]);
        assert_eq!(state, "blocked");
    }

    #[test]
    fn test_has_changes_requested_uses_latest_state_per_reviewer() {
        let reviews = vec![
            ReviewInfo {
                reviewer: "alice".to_string(),
                state: "CHANGES_REQUESTED".to_string(),
            },
            ReviewInfo {
                reviewer: "alice".to_string(),
                state: "APPROVED".to_string(),
            },
        ];
        assert!(!has_changes_requested(&reviews));
    }

    #[test]
    fn test_has_changes_requested_true_when_latest_state_is_changes_requested() {
        let reviews = vec![
            ReviewInfo {
                reviewer: "alice".to_string(),
                state: "APPROVED".to_string(),
            },
            ReviewInfo {
                reviewer: "bob".to_string(),
                state: "CHANGES_REQUESTED".to_string(),
            },
        ];
        assert!(has_changes_requested(&reviews));
    }

    #[test]
    fn test_non_required_warning_detected_from_optional_checks() {
        let checks = vec![
            WorkflowRunInfo {
                workflow_name: "Required CI".to_string(),
                run_id: 10,
                status: "completed".to_string(),
                conclusion: Some("success".to_string()),
                is_required: Some(true),
            },
            WorkflowRunInfo {
                workflow_name: "Optional Lint".to_string(),
                run_id: 11,
                status: "completed".to_string(),
                conclusion: Some("failure".to_string()),
                is_required: Some(false),
            },
        ];

        assert!(compute_non_required_checks_warning(&checks));
    }

    #[test]
    fn test_non_required_warning_hidden_when_required_checks_fail() {
        let checks = vec![
            WorkflowRunInfo {
                workflow_name: "Required CI".to_string(),
                run_id: 30,
                status: "completed".to_string(),
                conclusion: Some("failure".to_string()),
                is_required: Some(true),
            },
            WorkflowRunInfo {
                workflow_name: "Optional Lint".to_string(),
                run_id: 31,
                status: "completed".to_string(),
                conclusion: Some("failure".to_string()),
                is_required: Some(false),
            },
        ];

        assert!(!compute_non_required_checks_warning(&checks));
    }

    #[test]
    fn test_branch_pr_reference_serialization() {
        let pr = BranchPrReference {
            number: 123,
            title: "Test PR".to_string(),
            state: "OPEN".to_string(),
            url: Some("https://github.com/example/repo/pull/123".to_string()),
        };

        let json = serde_json::to_string(&pr).unwrap();
        assert!(json.contains("\"number\":123"));
        assert!(json.contains("\"state\":\"OPEN\""));
        assert!(json.contains("\"url\":\"https://github.com/example/repo/pull/123\""));
    }

    #[test]
    fn test_to_branch_pr_preflight_reports_behind_as_blocking() {
        let response = to_branch_pr_preflight("develop".to_string(), 0, 2);

        assert_eq!(response.base_branch, "develop");
        assert_eq!(response.ahead_by, 0);
        assert_eq!(response.behind_by, 2);
        assert_eq!(response.status, "behind");
        assert_eq!(
            response.blocking_reason.as_deref(),
            Some("Branch update required before creating a PR.")
        );
    }

    #[test]
    fn test_to_branch_pr_preflight_reports_diverged_as_blocking() {
        let response = to_branch_pr_preflight("develop".to_string(), 3, 1);

        assert_eq!(response.base_branch, "develop");
        assert_eq!(response.ahead_by, 3);
        assert_eq!(response.behind_by, 1);
        assert_eq!(response.status, "diverged");
        assert_eq!(
            response.blocking_reason.as_deref(),
            Some("Branch has diverged from base. Sync it before creating a PR.")
        );
    }

    #[test]
    fn test_to_branch_pr_preflight_reports_ahead_without_blocking() {
        let response = to_branch_pr_preflight("develop".to_string(), 4, 0);

        assert_eq!(response.status, "ahead");
        assert_eq!(response.blocking_reason, None);
    }

    #[test]
    fn test_fetch_branch_pr_preflight_uses_pr_base_and_refreshes_remote_refs() {
        let temp = setup_pr_preflight_repo();
        let repo = temp.path().join("repo");

        let response = fetch_branch_pr_preflight_impl(
            repo.to_string_lossy().to_string(),
            "feature/preflight".to_string(),
            None,
        )
        .unwrap();

        assert_eq!(response.base_branch, "origin/develop");
        assert_eq!(response.ahead_by, 0);
        assert!(response.behind_by > 0);
        assert_eq!(response.status, "behind");
        assert_eq!(
            response.blocking_reason.as_deref(),
            Some("Branch update required before creating a PR.")
        );
    }

    #[test]
    fn test_strip_known_remote_prefix_for_origin_and_custom_remote() {
        let remotes = vec![
            Remote::new("origin", "git@github.com:o/r.git"),
            Remote::new("upstream", "git@github.com:o/r.git"),
        ];

        assert_eq!(
            strip_known_remote_prefix("origin/feature/x", &remotes),
            "feature/x"
        );
        assert_eq!(
            strip_known_remote_prefix("upstream/feature/x", &remotes),
            "feature/x"
        );
        assert_eq!(
            strip_known_remote_prefix("fork/feature/x", &remotes),
            "fork/feature/x"
        );
    }

    #[test]
    fn test_extract_remote_host_https_and_ssh() {
        assert_eq!(
            extract_remote_host("https://github.com/example/repo.git"),
            Some("github.com".to_string())
        );
        assert_eq!(
            extract_remote_host("git@github.enterprise.local:example/repo.git"),
            Some("github.enterprise.local".to_string())
        );
    }

    #[test]
    fn test_extract_remote_host_invalid_and_file_scheme() {
        assert_eq!(extract_remote_host(""), None);
        assert_eq!(extract_remote_host("file:///tmp/repo.git"), None);
    }

    #[test]
    fn test_pr_merge_timeout_value() {
        assert_eq!(PR_MERGE_TIMEOUT.as_secs(), 15);
    }

    // ==========================================================
    // T001: PrStatusLiteSummary retrying field serialization
    // ==========================================================

    #[test]
    fn test_pr_status_lite_summary_retrying_serialization() {
        let summary_retrying = PrStatusLiteSummary {
            number: 1,
            state: "OPEN".to_string(),
            url: "https://example.com/1".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: None,
            merge_ui_state: "mergeable".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/a".to_string(),
            check_suites: vec![],
            retrying: true,
        };
        let json = serde_json::to_string(&summary_retrying).unwrap();
        assert!(json.contains("\"retrying\":true"));

        let summary_not_retrying = PrStatusLiteSummary {
            number: 2,
            state: "OPEN".to_string(),
            url: "https://example.com/2".to_string(),
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: None,
            merge_ui_state: "checking".to_string(),
            non_required_checks_warning: false,
            author: "bob".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/b".to_string(),
            check_suites: vec![],
            retrying: false,
        };
        let json = serde_json::to_string(&summary_not_retrying).unwrap();
        assert!(json.contains("\"retrying\":false"));
    }

    // ==========================================================
    // T002: PrRetryState management tests
    // ==========================================================

    #[test]
    fn test_pr_retry_state_new() {
        let state = PrRetryState::new();
        assert!(state.retrying);
        assert_eq!(state.retry_count, 0);
    }

    #[test]
    fn test_retry_state_in_cache_entry() {
        let mut entry = RepoPrStatusCacheEntry::default();
        assert!(entry.retry_states.is_empty());

        entry
            .retry_states
            .insert("feature/x".to_string(), PrRetryState::new());
        assert!(entry.retry_states.contains_key("feature/x"));
        assert!(entry.retry_states["feature/x"].retrying);
    }

    #[test]
    fn test_retry_constants() {
        assert_eq!(RETRY_MAX_ATTEMPTS, 5);
        assert_eq!(RETRY_INITIAL_INTERVAL.as_secs(), 2);
    }

    // ==========================================================
    // T003: Cache UNKNOWN protection tests
    // ==========================================================

    #[test]
    fn test_has_unknown_merge_status_mergeable_unknown() {
        let summary = PrStatusLiteSummary {
            number: 1,
            state: "OPEN".to_string(),
            url: "https://example.com/1".to_string(),
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: None,
            merge_ui_state: "checking".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/a".to_string(),
            check_suites: vec![],
            retrying: false,
        };
        assert!(has_unknown_merge_status(&summary));
    }

    #[test]
    fn test_has_unknown_merge_status_merge_state_unknown() {
        let summary = PrStatusLiteSummary {
            number: 1,
            state: "OPEN".to_string(),
            url: "https://example.com/1".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: Some("UNKNOWN".to_string()),
            merge_ui_state: "checking".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/a".to_string(),
            check_suites: vec![],
            retrying: false,
        };
        assert!(has_unknown_merge_status(&summary));
    }

    #[test]
    fn test_has_unknown_merge_status_known() {
        let summary = PrStatusLiteSummary {
            number: 1,
            state: "OPEN".to_string(),
            url: "https://example.com/1".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: Some("CLEAN".to_string()),
            merge_ui_state: "mergeable".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/a".to_string(),
            check_suites: vec![],
            retrying: false,
        };
        assert!(!has_unknown_merge_status(&summary));
    }

    #[test]
    fn test_cache_protection_preserves_known_values() {
        // Simulate: cache has MERGEABLE, new result has UNKNOWN
        let cached = PrStatusLiteSummary {
            number: 42,
            state: "OPEN".to_string(),
            url: "https://example.com/42".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: Some("CLEAN".to_string()),
            merge_ui_state: "mergeable".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/x".to_string(),
            check_suites: vec![],
            retrying: false,
        };

        let mut new_result = PrStatusLiteSummary {
            number: 42,
            state: "OPEN".to_string(),
            url: "https://example.com/42".to_string(),
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: Some("UNKNOWN".to_string()),
            merge_ui_state: "checking".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/x".to_string(),
            check_suites: vec![],
            retrying: false,
        };

        // Apply protection logic
        restore_unknown_merge_fields_from_cache(&mut new_result, &cached);

        assert_eq!(new_result.mergeable, "MERGEABLE");
        assert_eq!(new_result.merge_state_status, Some("CLEAN".to_string()));
        // Verify cached is unchanged
        assert_eq!(cached.mergeable, "MERGEABLE");
    }

    #[test]
    fn test_cache_protection_only_restores_unknown_fields() {
        // Simulate: new result has known mergeable but UNKNOWN merge_state_status.
        let cached = PrStatusLiteSummary {
            number: 42,
            state: "OPEN".to_string(),
            url: "https://example.com/42".to_string(),
            mergeable: "MERGEABLE".to_string(),
            merge_state_status: Some("CLEAN".to_string()),
            merge_ui_state: "mergeable".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/x".to_string(),
            check_suites: vec![],
            retrying: false,
        };

        let mut new_result = PrStatusLiteSummary {
            number: 42,
            state: "OPEN".to_string(),
            url: "https://example.com/42".to_string(),
            mergeable: "CONFLICTING".to_string(),
            merge_state_status: Some("UNKNOWN".to_string()),
            merge_ui_state: "conflicting".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/x".to_string(),
            check_suites: vec![],
            retrying: false,
        };

        restore_unknown_merge_fields_from_cache(&mut new_result, &cached);

        // Known field from latest response must be preserved.
        assert_eq!(new_result.mergeable, "CONFLICTING");
        // Only UNKNOWN field should be restored from cache.
        assert_eq!(new_result.merge_state_status, Some("CLEAN".to_string()));
    }

    #[test]
    fn test_cache_protection_allows_initial_unknown() {
        // When cache is empty (no previous value), UNKNOWN should be stored
        let new_result = PrStatusLiteSummary {
            number: 42,
            state: "OPEN".to_string(),
            url: "https://example.com/42".to_string(),
            mergeable: "UNKNOWN".to_string(),
            merge_state_status: None,
            merge_ui_state: "checking".to_string(),
            non_required_checks_warning: false,
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/x".to_string(),
            check_suites: vec![],
            retrying: false,
        };

        // No cached entry exists, so UNKNOWN should pass through
        assert!(has_unknown_merge_status(&new_result));
        assert_eq!(new_result.mergeable, "UNKNOWN");
    }

    #[test]
    fn test_apply_retry_state_overrides_marks_matching_branch() {
        let mut statuses = HashMap::from([(
            "feature/x".to_string(),
            Some(PrStatusLiteSummary {
                number: 42,
                state: "OPEN".to_string(),
                url: "https://example.com/42".to_string(),
                mergeable: "MERGEABLE".to_string(),
                merge_state_status: Some("CLEAN".to_string()),
                merge_ui_state: "mergeable".to_string(),
                non_required_checks_warning: false,
                author: "alice".to_string(),
                base_branch: "main".to_string(),
                head_branch: "feature/x".to_string(),
                check_suites: vec![],
                retrying: false,
            }),
        )]);
        let retry_states = HashMap::from([(
            "feature/x".to_string(),
            PrRetryState {
                retrying: true,
                retry_count: 2,
            },
        )]);

        apply_retry_state_overrides(&mut statuses, &retry_states);

        let summary = statuses["feature/x"].as_ref().unwrap();
        assert!(summary.retrying);
    }

    #[test]
    fn test_collect_unknown_branches_uses_raw_fetch_values() {
        let raw_statuses = HashMap::from([(
            "feature/x".to_string(),
            PrStatusLiteSummary {
                number: 42,
                state: "OPEN".to_string(),
                url: "https://example.com/42".to_string(),
                mergeable: "UNKNOWN".to_string(),
                merge_state_status: Some("UNKNOWN".to_string()),
                merge_ui_state: "checking".to_string(),
                non_required_checks_warning: false,
                author: "alice".to_string(),
                base_branch: "main".to_string(),
                head_branch: "feature/x".to_string(),
                check_suites: vec![],
                retrying: false,
            },
        )]);

        let mut final_statuses = raw_statuses.clone();
        final_statuses
            .get_mut("feature/x")
            .expect("entry should exist")
            .mergeable = "MERGEABLE".to_string();
        final_statuses
            .get_mut("feature/x")
            .expect("entry should exist")
            .merge_state_status = Some("CLEAN".to_string());

        let unknown_from_raw = collect_unknown_branches(&raw_statuses);
        let unknown_from_final = collect_unknown_branches(&final_statuses);
        assert_eq!(unknown_from_raw, vec!["feature/x".to_string()]);
        assert!(unknown_from_final.is_empty());
    }

    #[test]
    fn test_select_repo_merge_method_prefers_merge_commit() {
        assert_eq!(select_repo_merge_method(true, true, true), Some("merge"));
        assert_eq!(select_repo_merge_method(true, false, false), Some("merge"));
    }

    #[test]
    fn test_select_repo_merge_method_falls_back_to_squash_then_rebase() {
        assert_eq!(select_repo_merge_method(false, true, true), Some("squash"));
        assert_eq!(select_repo_merge_method(false, false, true), Some("rebase"));
    }

    #[test]
    fn test_select_repo_merge_method_returns_none_when_all_disabled() {
        assert_eq!(select_repo_merge_method(false, false, false), None);
    }
}
