//! Pull Request status commands (SPEC-d6949f99, SPEC-a9f2e3b1)

use crate::commands::project::resolve_repo_path_for_project_root;
use chrono::{DateTime, Utc};
use gwt_core::git::graphql;
use gwt_core::git::{
    is_gh_cli_authenticated, is_gh_cli_available, PrCache, PrListItem, PrStatusInfo, Remote,
    ReviewComment, ReviewInfo, WorkflowRunInfo,
};
use gwt_core::StructuredError;
use serde::Serialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ExitStatus, Stdio};
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

fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return None;
                }
                thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

fn read_pipe_to_string<T: Read>(mut pipe: T) -> String {
    let mut buf = String::new();
    let _ = pipe.read_to_string(&mut buf);
    buf
}

fn spawn_pipe_reader<T>(pipe: Option<T>) -> thread::JoinHandle<String>
where
    T: Read + Send + 'static,
{
    thread::spawn(move || pipe.map(read_pipe_to_string).unwrap_or_default())
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
fn has_unknown_merge_status(summary: &PrStatusLiteSummary) -> bool {
    summary.mergeable == "UNKNOWN"
        || summary
            .merge_state_status
            .as_deref()
            .map(|s| s == "UNKNOWN")
            .unwrap_or(false)
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
            let fetch_result =
                graphql::fetch_pr_statuses_with_meta(&repo_path, &still_unknown);

            match fetch_result {
                Ok(result) => {
                    let cache = pr_status_cache();
                    let mut guard =
                        cache.lock().unwrap_or_else(|poison| poison.into_inner());
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
                        let mut guard =
                            cache.lock().unwrap_or_else(|poison| poison.into_inner());
                        let entry = guard.repos.entry(repo_key.clone()).or_default();
                        entry.cooldown_until =
                            Some(Instant::now() + PR_STATUS_RATE_LIMIT_BACKOFF);
                    }
                    // Continue to next attempt
                }
            }

            // Check if all resolved
            {
                let cache = pr_status_cache();
                let guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
                if let Some(entry) = guard.repos.get(&repo_key) {
                    let any_still_retrying = branches_to_retry
                        .iter()
                        .any(|b| {
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

fn to_pr_status_summary(info: &PrStatusInfo) -> PrStatusLiteSummary {
    PrStatusLiteSummary {
        number: info.number,
        state: info.state.clone(),
        url: info.url.clone(),
        mergeable: info.mergeable.clone(),
        merge_state_status: info.merge_state_status.clone(),
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
    PrDetailResponse {
        number: info.number,
        title: info.title.clone(),
        state: info.state.clone(),
        url: info.url.clone(),
        mergeable: info.mergeable.clone(),
        merge_state_status: info.merge_state_status.clone(),
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
            for summary in statuses.values_mut().flatten() {
                if let Some(retry_state) = entry.retry_states.get(&summary.head_branch) {
                    summary.retrying = retry_state.retrying;
                }
            }
            return Ok(FetchPrStatusResult {
                response: PrStatusResponse {
                    statuses,
                    gh_status,
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
            let entry = guard.repos.entry(repo_key).or_default();
            if graphql::is_rate_limit_error(&error) {
                entry.cooldown_until = Some(Instant::now() + PR_STATUS_RATE_LIMIT_BACKOFF);
            }
            // Silent degrade: use stale cache if available, otherwise no statuses.
            let statuses = map_cached_statuses(&branches, &entry.statuses_by_head_branch);
            return Ok(FetchPrStatusResult {
                response: PrStatusResponse {
                    statuses,
                    gh_status,
                },
                unknown_branches: vec![],
                repo_path: None,
                repo_key: None,
            });
        }
    };

    // Write cache with UNKNOWN protection (FR-005):
    // If the new result has UNKNOWN merge fields but the cache already has
    // a known value, preserve the cached merge fields instead of regressing.
    let final_statuses = {
        let cache = pr_status_cache();
        let mut guard = cache.lock().unwrap_or_else(|poison| poison.into_inner());
        let entry = guard.repos.entry(repo_key.clone()).or_default();

        let mut merged = statuses_by_head_branch.clone();
        for (branch, new_summary) in &mut merged {
            if has_unknown_merge_status(new_summary) {
                if let Some(cached) = entry.statuses_by_head_branch.get(branch) {
                    if !has_unknown_merge_status(cached) {
                        // Preserve known merge fields from cache
                        new_summary.mergeable = cached.mergeable.clone();
                        new_summary.merge_state_status = cached.merge_state_status.clone();
                    }
                }
            }
        }

        entry.statuses_by_head_branch = merged.clone();
        entry.fetched_at = Some(now);
        entry.cooldown_until = cooldown_until;
        merged
    };

    // Detect UNKNOWN branches and mark retrying (FR-001)
    let unknown_branches: Vec<String> = final_statuses
        .iter()
        .filter(|(_, s)| has_unknown_merge_status(s))
        .map(|(branch, _)| branch.clone())
        .collect();

    let mut statuses = map_cached_statuses(&branches, &final_statuses);

    // Set retrying flag on UNKNOWN PR statuses
    if !unknown_branches.is_empty() {
        for summary in statuses.values_mut().flatten() {
            if unknown_branches.contains(&summary.head_branch) {
                summary.retrying = true;
            }
        }
    }

    Ok(FetchPrStatusResult {
        response: PrStatusResponse {
            statuses,
            gh_status,
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

/// Update a PR branch with the latest base branch changes (SPEC-de3290fc T008)
fn update_pr_branch_impl(project_path: String, pr_number: u64) -> Result<String, String> {
    use gwt_core::git::gh_cli::gh_command;
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

    let mut child = gh_command()
        .args([
            "api",
            "-X",
            "PUT",
            &format!("/repos/{owner}/{repo}/pulls/{pr_number}/update-branch"),
        ])
        .current_dir(&repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute gh api: {}", e))?;

    let stdout_handle = spawn_pipe_reader(child.stdout.take());
    let stderr_handle = spawn_pipe_reader(child.stderr.take());

    let status = match wait_with_timeout(&mut child, PR_UPDATE_BRANCH_TIMEOUT) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let stderr = stderr_handle.join().unwrap_or_default();
            let detail = stderr.trim();
            if detail.is_empty() {
                return Err(format!(
                    "Failed to update PR branch: gh api timed out after {}s",
                    PR_UPDATE_BRANCH_TIMEOUT.as_secs()
                ));
            }
            return Err(format!(
                "Failed to update PR branch: gh api timed out after {}s: {}",
                PR_UPDATE_BRANCH_TIMEOUT.as_secs(),
                detail
            ));
        }
    };

    let _stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    if !status.success() {
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

/// Merge a pull request via GitHub REST API (SPEC-merge-pr FR-004)
fn merge_pull_request_impl(project_path: String, pr_number: u64) -> Result<String, String> {
    use gwt_core::git::gh_cli::gh_command;
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

    let mut child = gh_command()
        .args([
            "api",
            "-X",
            "PUT",
            &format!("/repos/{owner}/{repo}/pulls/{pr_number}/merge"),
        ])
        .current_dir(&repo_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to execute gh api: {}", e))?;

    let stdout_handle = spawn_pipe_reader(child.stdout.take());
    let stderr_handle = spawn_pipe_reader(child.stderr.take());

    let status = match wait_with_timeout(&mut child, PR_MERGE_TIMEOUT) {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_handle.join();
            let stderr = stderr_handle.join().unwrap_or_default();
            let detail = stderr.trim();
            if detail.is_empty() {
                return Err(format!(
                    "Failed to merge PR: gh api timed out after {}s",
                    PR_MERGE_TIMEOUT.as_secs()
                ));
            }
            return Err(format!(
                "Failed to merge PR: gh api timed out after {}s: {}",
                PR_MERGE_TIMEOUT.as_secs(),
                detail
            ));
        }
    };

    let _stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();

    if !status.success() {
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
// PR Dashboard commands (SPEC-prlist)
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
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"statuses\""));
        assert!(json.contains("\"ghStatus\""));
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
        };
        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"statuses\":{}"));
        assert!(json.contains("\"ghStatus\""));
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
            author: "alice".to_string(),
            base_branch: "main".to_string(),
            head_branch: "feature/x".to_string(),
            check_suites: vec![],
            retrying: false,
        };

        // Apply protection logic
        if has_unknown_merge_status(&new_result) && !has_unknown_merge_status(&cached) {
            new_result.mergeable = cached.mergeable.clone();
            new_result.merge_state_status = cached.merge_state_status.clone();
        }

        assert_eq!(new_result.mergeable, "MERGEABLE");
        assert_eq!(
            new_result.merge_state_status,
            Some("CLEAN".to_string())
        );
        // Verify cached is unchanged
        assert_eq!(cached.mergeable, "MERGEABLE");
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
}
