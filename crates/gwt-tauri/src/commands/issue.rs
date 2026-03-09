//! GitHub Issue commands (gwt-spec issue)

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::{AppState, IssueListCacheEntry};
use gwt_core::ai::{
    classify_issue_prefix as core_classify_issue_prefix, format_error_for_display, AIClient,
};
use gwt_core::config::ProfilesConfig;
use gwt_core::git::{
    create_linked_branch, fetch_issue_detail, fetch_issues_with_options, find_branch_for_issue,
    find_branches_for_issues, get_spec_issue_detail, is_gh_cli_authenticated, is_gh_cli_available,
};
use gwt_core::worktree::WorktreeManager;
use gwt_core::StructuredError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::Manager;

const ISSUE_LIST_CACHE_TTL_MS: i64 = 120_000;
const ISSUE_LIST_CACHE_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;
const ISSUE_LIST_INFLIGHT_WAIT_MS: u64 = 5_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IssueCategory {
    All,
    Issues,
    Specs,
}

impl IssueCategory {
    fn as_str(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Issues => "issues",
            Self::Specs => "specs",
        }
    }
}

/// Response for fetch_github_issues (FR-010a)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FetchIssuesResponse {
    pub issues: Vec<IssueInfo>,
    pub has_next_page: bool,
}

/// Serializable label info for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelInfo {
    pub name: String,
    pub color: String,
}

/// Serializable assignee info for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssigneeInfo {
    pub login: String,
    pub avatar_url: String,
}

/// Serializable milestone info for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MilestoneInfo {
    pub title: String,
    pub number: u32,
}

/// Serializable issue info for the frontend
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueInfo {
    pub number: u64,
    pub title: String,
    pub updated_at: String,
    pub labels: Vec<LabelInfo>,
    pub body: Option<String>,
    pub state: String,
    pub html_url: String,
    pub assignees: Vec<AssigneeInfo>,
    pub comments_count: u32,
    pub milestone: Option<MilestoneInfo>,
}

/// Branch-linked issue info for Worktree Summary.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchLinkedIssueInfo {
    pub number: u64,
    pub title: String,
    pub updated_at: String,
    pub labels: Vec<String>,
    pub url: String,
}

/// gh CLI status (FR-011a)
#[derive(Debug, Clone, Serialize)]
pub struct GhCliStatus {
    pub available: bool,
    pub authenticated: bool,
}

/// Rollback result (FR-014)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RollbackResult {
    pub local_deleted: bool,
    pub remote_deleted: bool,
    pub error: Option<String>,
}

/// Branch mapping result for bulk issue lookup.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IssueBranchMatch {
    pub issue_number: u64,
    pub branch_name: String,
}

fn parse_issue_category(value: Option<String>) -> IssueCategory {
    match value
        .as_deref()
        .map(str::trim)
        .unwrap_or("issues")
        .to_ascii_lowercase()
        .as_str()
    {
        "specs" => IssueCategory::Specs,
        "all" => IssueCategory::All,
        _ => IssueCategory::Issues,
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn issue_cache_file_path(repo_path: &Path) -> PathBuf {
    let canonical = repo_path
        .canonicalize()
        .unwrap_or_else(|_| repo_path.to_path_buf());
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let hash = format!("{digest:x}");
    let short_hash = &hash[..16];

    let base = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    base.join(".gwt")
        .join("cache")
        .join("issues")
        .join(format!("{short_hash}.json"))
}

fn load_issue_disk_cache(repo_path: &Path) -> Option<HashMap<String, IssueListCacheEntry>> {
    let path = issue_cache_file_path(repo_path);
    let data = fs::read_to_string(path).ok()?;
    serde_json::from_str::<HashMap<String, IssueListCacheEntry>>(&data).ok()
}

fn save_issue_disk_cache(repo_path: &Path, entries: &HashMap<String, IssueListCacheEntry>) {
    let path = issue_cache_file_path(repo_path);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(entries) {
        let _ = fs::write(path, json);
    }
}

fn prune_issue_cache_entries(
    entries: &mut HashMap<String, IssueListCacheEntry>,
    now_ms: i64,
) -> bool {
    let before_len = entries.len();
    entries.retain(|_, entry| now_ms - entry.fetched_at_millis <= ISSUE_LIST_CACHE_RETENTION_MS);
    entries.len() != before_len
}

fn issue_cache_key(
    page: u32,
    per_page: u32,
    state: &str,
    category: IssueCategory,
    include_body: bool,
) -> String {
    format!(
        "page={page}&per_page={per_page}&state={state}&category={}&include_body={include_body}",
        category.as_str()
    )
}

fn decode_cached_issue_response(entry: &IssueListCacheEntry) -> Option<FetchIssuesResponse> {
    serde_json::from_str::<FetchIssuesResponse>(&entry.response_json).ok()
}

fn try_get_issue_cache(
    state: &AppState,
    repo_path: &Path,
    repo_key: &str,
    cache_key: &str,
    now_ms: i64,
) -> Option<FetchIssuesResponse> {
    let mut snapshot_to_persist: Option<HashMap<String, IssueListCacheEntry>> = None;
    let mut memory_hit: Option<FetchIssuesResponse> = None;
    {
        let mut guard = state.project_issue_list_cache.lock().ok()?;
        if let Some(repo_map) = guard.get_mut(repo_key) {
            if prune_issue_cache_entries(repo_map, now_ms) {
                snapshot_to_persist = Some(repo_map.clone());
            }
            if let Some(entry) = repo_map.get(cache_key) {
                if now_ms - entry.fetched_at_millis <= ISSUE_LIST_CACHE_TTL_MS {
                    memory_hit = decode_cached_issue_response(entry);
                }
            }
        }
    }
    if let Some(entries) = snapshot_to_persist.as_ref() {
        save_issue_disk_cache(repo_path, entries);
    }
    if let Some(hit) = memory_hit {
        return Some(hit);
    }

    let mut disk_map = load_issue_disk_cache(repo_path)?;
    let changed = prune_issue_cache_entries(&mut disk_map, now_ms);
    if changed {
        save_issue_disk_cache(repo_path, &disk_map);
    }

    if let Ok(mut guard) = state.project_issue_list_cache.lock() {
        let repo_entry = guard.entry(repo_key.to_string()).or_default();
        for (k, v) in &disk_map {
            repo_entry.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }

    let entry = disk_map.get(cache_key)?;
    if now_ms - entry.fetched_at_millis > ISSUE_LIST_CACHE_TTL_MS {
        return None;
    }
    decode_cached_issue_response(entry)
}

fn put_issue_cache(
    state: &AppState,
    repo_path: &Path,
    repo_key: &str,
    cache_key: &str,
    response: &FetchIssuesResponse,
    now_ms: i64,
) {
    let Ok(response_json) = serde_json::to_string(response) else {
        return;
    };
    let new_entry = IssueListCacheEntry {
        fetched_at_millis: now_ms,
        response_json,
    };
    let mut snapshot_to_persist: Option<HashMap<String, IssueListCacheEntry>> = None;
    if let Ok(mut guard) = state.project_issue_list_cache.lock() {
        let repo_entry = guard.entry(repo_key.to_string()).or_default();
        repo_entry.insert(cache_key.to_string(), new_entry);
        prune_issue_cache_entries(repo_entry, now_ms);
        snapshot_to_persist = Some(repo_entry.clone());
    }
    if let Some(entries) = snapshot_to_persist.as_ref() {
        save_issue_disk_cache(repo_path, entries);
    }
}

fn invalidate_issue_cache_for_repo(state: &AppState, repo_path: &Path, repo_key: &str) {
    if let Ok(mut guard) = state.project_issue_list_cache.lock() {
        guard.remove(repo_key);
    }
    let path = issue_cache_file_path(repo_path);
    let _ = fs::remove_file(path);
}

fn mark_issue_cache_inflight(state: &AppState, inflight_key: &str) -> bool {
    if let Ok(mut set) = state.project_issue_list_inflight.lock() {
        if set.contains(inflight_key) {
            return false;
        }
        set.insert(inflight_key.to_string());
        return true;
    }
    true
}

fn clear_issue_cache_inflight(state: &AppState, inflight_key: &str) {
    if let Ok(mut set) = state.project_issue_list_inflight.lock() {
        set.remove(inflight_key);
    }
}

fn wait_for_issue_cache_inflight(state: &AppState, inflight_key: &str) {
    let mut waited_ms: u64 = 0;
    while waited_ms < ISSUE_LIST_INFLIGHT_WAIT_MS {
        let still_inflight = state
            .project_issue_list_inflight
            .lock()
            .map(|set| set.contains(inflight_key))
            .unwrap_or(false);
        if !still_inflight {
            break;
        }
        thread::sleep(Duration::from_millis(10));
        waited_ms += 10;
    }
}

/// Convert a core GitHubIssue to the serializable IssueInfo
fn issue_to_info(issue: gwt_core::git::GitHubIssue) -> IssueInfo {
    IssueInfo {
        number: issue.number,
        title: issue.title,
        updated_at: issue.updated_at,
        labels: issue
            .labels
            .into_iter()
            .map(|l| LabelInfo {
                name: l.name,
                color: l.color,
            })
            .collect(),
        body: issue.body,
        state: normalize_issue_state(&issue.state),
        html_url: issue.html_url,
        assignees: issue
            .assignees
            .into_iter()
            .map(|a| AssigneeInfo {
                login: a.login,
                avatar_url: a.avatar_url,
            })
            .collect(),
        comments_count: issue.comments_count,
        milestone: issue.milestone.map(|m| MilestoneInfo {
            title: m.title,
            number: m.number,
        }),
    }
}

fn normalize_issue_state(state: &str) -> String {
    if state.eq_ignore_ascii_case("closed") {
        "closed".to_string()
    } else {
        "open".to_string()
    }
}

/// Fetch GitHub issues with pagination (FR-010) – blocking impl
#[allow(clippy::too_many_arguments)]
fn fetch_github_issues_impl(
    project_path: String,
    page: u32,
    per_page: u32,
    state: Option<String>,
    category: Option<String>,
    include_body: Option<bool>,
    force_refresh: Option<bool>,
    app_state: &AppState,
) -> Result<FetchIssuesResponse, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_issues"))?;
    let state = state.unwrap_or_else(|| "open".to_string());
    let category = parse_issue_category(category);
    let include_body = include_body.unwrap_or(false);
    let cache_enabled = !include_body;
    let force_refresh = force_refresh.unwrap_or(false);
    let repo_key = repo_path.to_string_lossy().to_string();
    let cache_key = issue_cache_key(page, per_page, &state, category, include_body);
    let inflight_key = format!("{repo_key}::{cache_key}");
    let now_ms = now_millis();

    if cache_enabled && force_refresh {
        invalidate_issue_cache_for_repo(app_state, &repo_path, &repo_key);
    } else if cache_enabled {
        if let Some(hit) = try_get_issue_cache(app_state, &repo_path, &repo_key, &cache_key, now_ms)
        {
            return Ok(hit);
        }
    }

    let mut fetch_owner = false;
    if cache_enabled {
        fetch_owner = mark_issue_cache_inflight(app_state, &inflight_key);
        if !fetch_owner {
            wait_for_issue_cache_inflight(app_state, &inflight_key);
            if let Some(hit) =
                try_get_issue_cache(app_state, &repo_path, &repo_key, &cache_key, now_millis())
            {
                return Ok(hit);
            }
        }
    }

    let fetch_result = fetch_issues_with_options(
        &repo_path,
        page,
        per_page,
        &state,
        include_body,
        category.as_str(),
    )
    .map_err(|e| StructuredError::internal(&e, "fetch_github_issues"))
    .map(|result| FetchIssuesResponse {
        issues: result.issues.into_iter().map(issue_to_info).collect(),
        has_next_page: result.has_next_page,
    });

    let result = match fetch_result {
        Ok(response) => {
            if cache_enabled {
                put_issue_cache(
                    app_state,
                    &repo_path,
                    &repo_key,
                    &cache_key,
                    &response,
                    now_millis(),
                );
            }
            Ok(response)
        }
        Err(err) => Err(err),
    };

    if cache_enabled && fetch_owner {
        clear_issue_cache_inflight(app_state, &inflight_key);
    }

    result
}

/// Fetch GitHub issues with pagination (FR-010)
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn fetch_github_issues(
    project_path: String,
    page: u32,
    per_page: u32,
    state: Option<String>,
    category: Option<String>,
    include_body: Option<bool>,
    force_refresh: Option<bool>,
    app: tauri::AppHandle<tauri::Wry>,
) -> Result<FetchIssuesResponse, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        let app_state = app.state::<AppState>();
        fetch_github_issues_impl(
            project_path,
            page,
            per_page,
            state,
            category,
            include_body,
            force_refresh,
            &app_state,
        )
    })
    .await
    .map_err(|e| {
        StructuredError::internal(&format!("Task join failed: {e}"), "fetch_github_issues")
    })?
}

/// Fetch a single GitHub issue detail – blocking impl
fn fetch_github_issue_detail_impl(
    project_path: String,
    issue_number: u64,
) -> Result<IssueInfo, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_issue_detail"))?;

    let issue = fetch_issue_detail(&repo_path, issue_number)
        .map_err(|e| StructuredError::internal(&e, "fetch_github_issue_detail"))?;
    Ok(issue_to_info(issue))
}

/// Fetch a single GitHub issue detail
#[tauri::command]
pub async fn fetch_github_issue_detail(
    project_path: String,
    issue_number: u64,
) -> Result<IssueInfo, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        fetch_github_issue_detail_impl(project_path, issue_number)
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Task join failed: {e}"),
            "fetch_github_issue_detail",
        )
    })?
}

fn extract_issue_number_from_branch(branch: &str) -> Option<u64> {
    let trimmed = branch.trim();
    if trimmed.is_empty() {
        return None;
    }

    for segment in trimmed.split('/') {
        let lower = segment.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("issue-") else {
            continue;
        };
        let digits: String = rest.chars().take_while(|ch| ch.is_ascii_digit()).collect();
        if !digits.is_empty() {
            if let Ok(number) = digits.parse::<u64>() {
                return Some(number);
            }
        }
    }
    None
}

fn is_issue_not_found_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    lower.contains("could not resolve to an issue")
        || (lower.contains("issue with the number") && lower.contains("(repository.issue)"))
}

/// Fetch issue linked to branch naming pattern (`issue-<number>`) – blocking impl
fn fetch_branch_linked_issue_impl(
    project_path: String,
    branch: String,
) -> Result<Option<BranchLinkedIssueInfo>, StructuredError> {
    let Some(issue_number) = extract_issue_number_from_branch(&branch) else {
        return Ok(None);
    };

    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "fetch_branch_linked_issue"))?;

    match get_spec_issue_detail(&repo_path, issue_number) {
        Ok(detail) => Ok(Some(BranchLinkedIssueInfo {
            number: detail.number,
            title: detail.title,
            updated_at: detail.updated_at,
            labels: detail.labels,
            url: detail.url,
        })),
        Err(err) if is_issue_not_found_error(&err) => Ok(None),
        Err(err) => Err(StructuredError::internal(&err, "fetch_branch_linked_issue")),
    }
}

/// Fetch issue linked to branch naming pattern (`issue-<number>`).
#[tauri::command]
pub async fn fetch_branch_linked_issue(
    project_path: String,
    branch: String,
) -> Result<Option<BranchLinkedIssueInfo>, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        fetch_branch_linked_issue_impl(project_path, branch)
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Task join failed: {e}"),
            "fetch_branch_linked_issue",
        )
    })?
}

/// Check gh CLI availability and authentication (FR-011)
#[tauri::command]
pub fn check_gh_cli_status(_project_path: String) -> Result<GhCliStatus, StructuredError> {
    let available = is_gh_cli_available();
    let authenticated = if available {
        is_gh_cli_authenticated()
    } else {
        false
    };

    Ok(GhCliStatus {
        available,
        authenticated,
    })
}

/// Find an existing branch for a given issue (FR-012)
#[tauri::command]
pub fn find_existing_issue_branch(
    project_path: String,
    issue_number: u64,
) -> Result<Option<String>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "find_existing_issue_branch"))?;

    find_branch_for_issue(&repo_path, issue_number)
        .map_err(|e| StructuredError::internal(&e, "find_existing_issue_branch"))
}

/// Bulk lookup of existing issue branches for list rendering – blocking impl
fn find_existing_issue_branches_bulk_impl(
    project_path: String,
    issue_numbers: Vec<u64>,
) -> Result<Vec<IssueBranchMatch>, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "find_existing_issue_branches_bulk"))?;

    let found = find_branches_for_issues(&repo_path, &issue_numbers)
        .map_err(|e| StructuredError::internal(&e, "find_existing_issue_branches_bulk"))?;

    let mut matches: Vec<IssueBranchMatch> = found
        .into_iter()
        .map(|(issue_number, branch_name)| IssueBranchMatch {
            issue_number,
            branch_name,
        })
        .collect();
    matches.sort_by_key(|m| m.issue_number);
    Ok(matches)
}

/// Bulk lookup of existing issue branches for list rendering.
#[tauri::command]
pub async fn find_existing_issue_branches_bulk(
    project_path: String,
    issue_numbers: Vec<u64>,
) -> Result<Vec<IssueBranchMatch>, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        find_existing_issue_branches_bulk_impl(project_path, issue_numbers)
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Task join failed: {e}"),
            "find_existing_issue_branches_bulk",
        )
    })?
}

/// Link a branch to a GitHub issue via `gh issue develop` (FR-013)
#[tauri::command]
pub fn link_branch_to_issue(
    project_path: String,
    issue_number: u64,
    branch_name: String,
) -> Result<(), StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "link_branch_to_issue"))?;

    create_linked_branch(&repo_path, issue_number, &branch_name)
        .map_err(|e| StructuredError::internal(&e, "link_branch_to_issue"))
}

/// Rollback an issue-linked branch (FR-014)
///
/// Deletes local branch and optionally the remote branch.
#[tauri::command]
pub fn rollback_issue_branch(
    project_path: String,
    branch_name: String,
    delete_remote: bool,
) -> Result<RollbackResult, StructuredError> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "rollback_issue_branch"))?;

    // Local rollback must remove worktree first, then delete the branch.
    let manager = WorktreeManager::new(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "rollback_issue_branch"))?;
    let (local_deleted, local_error) = match manager.cleanup_branch(&branch_name, true, true) {
        Ok(()) => (true, None),
        Err(err) => (false, Some(err.to_string())),
    };

    // Delete remote branch if requested (FR-014a)
    let (remote_deleted, remote_error) = if delete_remote {
        let remote_output = gwt_core::process::command("git")
            .args(["push", "origin", "--delete", &branch_name])
            .current_dir(&repo_path)
            .output()
            .map_err(|e| {
                StructuredError::internal(
                    &format!("Failed to execute git push --delete: {}", e),
                    "rollback_issue_branch",
                )
            })?;

        if remote_output.status.success() {
            (true, None)
        } else {
            let stderr = String::from_utf8_lossy(&remote_output.stderr).to_string();
            // FR-029b: remote deletion failure is not fatal
            (false, Some(stderr))
        }
    } else {
        (false, None)
    };

    let error = match (local_error, remote_error) {
        (None, None) => None,
        (Some(local), None) => Some(format!("Local cleanup warning: {}", local)),
        (None, Some(remote)) => Some(remote),
        (Some(local), Some(remote)) => Some(format!(
            "Local cleanup warning: {}\nRemote cleanup warning: {}",
            local, remote
        )),
    };

    Ok(RollbackResult {
        local_deleted,
        remote_deleted,
        error,
    })
}

/// AI-based issue branch prefix classification result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassifyResult {
    /// "ok" | "ai-not-configured" | "error"
    pub status: String,
    pub prefix: Option<String>,
    pub error: Option<String>,
}

/// Classify a GitHub issue into a branch prefix using AI.
#[tauri::command]
pub fn classify_issue_branch_prefix(
    title: String,
    labels: Vec<String>,
    body: Option<String>,
) -> Result<ClassifyResult, StructuredError> {
    let profiles = ProfilesConfig::load()
        .map_err(|e| StructuredError::from_gwt_error(&e, "classify_issue_branch_prefix"))?;
    let ai = profiles.resolve_active_ai_settings();
    let Some(settings) = ai.resolved else {
        return Ok(ClassifyResult {
            status: "ai-not-configured".to_string(),
            prefix: None,
            error: None,
        });
    };

    let client = AIClient::new(settings)
        .map_err(|e| StructuredError::internal(&e.to_string(), "classify_issue_branch_prefix"))?;
    match core_classify_issue_prefix(&client, &title, &labels, body.as_deref()) {
        Ok(prefix) => Ok(ClassifyResult {
            status: "ok".to_string(),
            prefix: Some(prefix),
            error: None,
        }),
        Err(err) => Ok(ClassifyResult {
            status: "error".to_string(),
            prefix: None,
            error: Some(format_error_for_display(&err)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================================
    // FR-010: FetchIssuesResponse serialization tests
    // ==========================================================

    #[test]
    fn test_fetch_issues_response_serialization() {
        let response = FetchIssuesResponse {
            issues: vec![IssueInfo {
                number: 42,
                title: "Fix login bug".to_string(),
                updated_at: "2025-01-25T10:00:00Z".to_string(),
                labels: vec![LabelInfo {
                    name: "bug".to_string(),
                    color: "d73a4a".to_string(),
                }],
                body: Some("Issue body".to_string()),
                state: "OPEN".to_string(),
                html_url: "https://github.com/user/repo/issues/42".to_string(),
                assignees: vec![],
                comments_count: 0,
                milestone: None,
            }],
            has_next_page: true,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"hasNextPage\":true"));
        assert!(json.contains("\"number\":42"));
        assert!(json.contains("\"updatedAt\":"));
        assert!(json.contains("\"state\":\"OPEN\""));
    }

    #[test]
    fn test_fetch_issues_response_empty() {
        let response = FetchIssuesResponse {
            issues: vec![],
            has_next_page: false,
        };

        let json = serde_json::to_string(&response).unwrap();
        assert!(json.contains("\"issues\":[]"));
        assert!(json.contains("\"hasNextPage\":false"));
    }

    #[test]
    fn test_issue_info_extended_fields_serialization() {
        let info = IssueInfo {
            number: 42,
            title: "Test".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec![LabelInfo {
                name: "bug".to_string(),
                color: "d73a4a".to_string(),
            }],
            body: Some("body".to_string()),
            state: "OPEN".to_string(),
            html_url: "https://github.com/user/repo/issues/42".to_string(),
            assignees: vec![AssigneeInfo {
                login: "octocat".to_string(),
                avatar_url: "https://avatars.example.com/1".to_string(),
            }],
            comments_count: 5,
            milestone: Some(MilestoneInfo {
                title: "v1.0".to_string(),
                number: 1,
            }),
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"body\":\"body\""));
        assert!(json.contains("\"state\":\"OPEN\""));
        assert!(json.contains("\"htmlUrl\":"));
        assert!(json.contains("\"commentsCount\":5"));
        assert!(json.contains("\"login\":\"octocat\""));
        assert!(json.contains("\"avatarUrl\":"));
    }

    #[test]
    fn test_issue_to_info_normalizes_state_to_lowercase() {
        let issue = gwt_core::git::GitHubIssue {
            number: 42,
            title: "Test".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec![],
            body: None,
            state: "CLOSED".to_string(),
            html_url: "https://github.com/user/repo/issues/42".to_string(),
            assignees: vec![],
            comments_count: 0,
            milestone: None,
        };

        let info = issue_to_info(issue);
        assert_eq!(info.state, "closed");
    }

    // ==========================================================
    // FR-011: GhCliStatus serialization tests
    // ==========================================================

    #[test]
    fn test_gh_cli_status_serialization() {
        let status = GhCliStatus {
            available: true,
            authenticated: true,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"available\":true"));
        assert!(json.contains("\"authenticated\":true"));
    }

    #[test]
    fn test_gh_cli_status_unavailable() {
        let status = GhCliStatus {
            available: false,
            authenticated: false,
        };

        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("\"available\":false"));
        assert!(json.contains("\"authenticated\":false"));
    }

    // ==========================================================
    // FR-014: RollbackResult serialization tests
    // ==========================================================

    #[test]
    fn test_rollback_result_success() {
        let result = RollbackResult {
            local_deleted: true,
            remote_deleted: true,
            error: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"localDeleted\":true"));
        assert!(json.contains("\"remoteDeleted\":true"));
        assert!(json.contains("\"error\":null"));
    }

    #[test]
    fn test_rollback_result_with_remote_error() {
        let result = RollbackResult {
            local_deleted: true,
            remote_deleted: false,
            error: Some("remote branch not found".to_string()),
        };

        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains("\"localDeleted\":true"));
        assert!(json.contains("\"remoteDeleted\":false"));
        assert!(json.contains("remote branch not found"));
    }

    // ==========================================================
    // IssueInfo serialization tests
    // ==========================================================

    #[test]
    fn test_issue_info_with_labels() {
        let info = IssueInfo {
            number: 42,
            title: "Fix bug".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec![
                LabelInfo {
                    name: "bug".to_string(),
                    color: "d73a4a".to_string(),
                },
                LabelInfo {
                    name: "urgent".to_string(),
                    color: "ff0000".to_string(),
                },
            ],
            body: None,
            state: "OPEN".to_string(),
            html_url: String::new(),
            assignees: vec![],
            comments_count: 0,
            milestone: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"name\":\"bug\""));
        assert!(json.contains("\"name\":\"urgent\""));
    }

    #[test]
    fn test_issue_info_empty_labels() {
        let info = IssueInfo {
            number: 1,
            title: "No labels".to_string(),
            updated_at: "2025-01-25T10:00:00Z".to_string(),
            labels: vec![],
            body: None,
            state: "OPEN".to_string(),
            html_url: String::new(),
            assignees: vec![],
            comments_count: 0,
            milestone: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        assert!(json.contains("\"labels\":[]"));
    }

    #[test]
    fn test_branch_linked_issue_info_serialization() {
        let issue = BranchLinkedIssueInfo {
            number: 1097,
            title: "Rework tabs".to_string(),
            updated_at: "2026-02-17T00:00:00Z".to_string(),
            labels: vec!["enhancement".to_string()],
            url: "https://github.com/example/repo/issues/1097".to_string(),
        };

        let json = serde_json::to_string(&issue).unwrap();
        assert!(json.contains("\"updatedAt\""));
        assert!(json.contains("\"url\":\"https://github.com/example/repo/issues/1097\""));
    }

    #[test]
    fn test_extract_issue_number_from_branch_variants() {
        assert_eq!(
            extract_issue_number_from_branch("feature/issue-1097"),
            Some(1097)
        );
        assert_eq!(
            extract_issue_number_from_branch("origin/bugfix/issue-42-something"),
            Some(42)
        );
        assert_eq!(extract_issue_number_from_branch("hotfix/ISSUE-9"), Some(9));
    }

    #[test]
    fn test_extract_issue_number_from_branch_absent() {
        assert_eq!(extract_issue_number_from_branch("feature/new-ui"), None);
        assert_eq!(
            extract_issue_number_from_branch("feature/noissue-123"),
            None
        );
        assert_eq!(extract_issue_number_from_branch("feature/reissue-42"), None);
        assert_eq!(extract_issue_number_from_branch(""), None);
    }

    #[test]
    fn test_is_issue_not_found_error() {
        assert!(is_issue_not_found_error(
            "gh issue view failed: could not resolve to an issue"
        ));
        assert!(is_issue_not_found_error(
            "gh issue view failed: GraphQL: Could not resolve to an issue with the number of 1097. (repository.issue)"
        ));
        assert!(!is_issue_not_found_error(
            "gh issue view failed: HTTP 404: Not Found"
        ));
        assert!(!is_issue_not_found_error(
            "gh issue view failed: GraphQL: Could not resolve to a Repository with the name 'org/repo'. (repository)"
        ));
        assert!(!is_issue_not_found_error("permission denied"));
    }

    #[test]
    fn test_parse_issue_category_defaults_to_issues() {
        assert_eq!(parse_issue_category(None), IssueCategory::Issues);
        assert_eq!(
            parse_issue_category(Some("unknown".to_string())),
            IssueCategory::Issues
        );
    }

    #[test]
    fn test_parse_issue_category_variants() {
        assert_eq!(
            parse_issue_category(Some("specs".to_string())),
            IssueCategory::Specs
        );
        assert_eq!(
            parse_issue_category(Some("all".to_string())),
            IssueCategory::All
        );
    }

    #[test]
    fn test_issue_cache_key_contains_category_and_body_flag() {
        let key = issue_cache_key(1, 30, "open", IssueCategory::Specs, false);
        assert!(key.contains("category=specs"));
        assert!(key.contains("include_body=false"));
    }

    #[test]
    fn test_prune_issue_cache_entries_removes_stale_rows() {
        let now = 1_000_000_i64;
        let mut entries = HashMap::new();
        entries.insert(
            "fresh".to_string(),
            IssueListCacheEntry {
                fetched_at_millis: now - 1_000,
                response_json: "{}".to_string(),
            },
        );
        entries.insert(
            "stale".to_string(),
            IssueListCacheEntry {
                fetched_at_millis: now - ISSUE_LIST_CACHE_RETENTION_MS - 1,
                response_json: "{}".to_string(),
            },
        );

        let changed = prune_issue_cache_entries(&mut entries, now);
        assert!(changed);
        assert!(entries.contains_key("fresh"));
        assert!(!entries.contains_key("stale"));
    }
}
