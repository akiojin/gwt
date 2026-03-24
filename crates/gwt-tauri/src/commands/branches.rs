//! Branch management commands

use std::{
    collections::{HashMap, HashSet},
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use gwt_core::{
    config::{agent_has_hook_support, infer_agent_status, AgentStatus, Session},
    git::{fetch_issue_detail, is_bare_repository, Branch, Remote},
    terminal::pane::PaneStatus,
    worktree::WorktreeManager,
    StructuredError,
};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};
use tracing::{error, instrument, warn};

use crate::{
    commands::{
        issue::FetchIssuesResponse, project::resolve_repo_path_for_project_root,
        terminal::capture_scrollback_tail_from_state,
    },
    state::AppState,
};

const LIST_WORKTREE_BRANCHES_WARN_THRESHOLD: Duration = Duration::from_millis(300);
const BRANCH_INVENTORY_INFLIGHT_WAIT_MS: u64 = 30_000;

/// Serializable branch info for the frontend
#[derive(Debug, Clone, Serialize)]
pub struct BranchInfo {
    pub name: String,
    pub display_name: Option<String>,
    pub commit: String,
    pub is_current: bool,
    pub is_agent_running: bool,
    pub agent_status: String,
    pub has_remote: bool,
    pub upstream: Option<String>,
    pub ahead: usize,
    pub behind: usize,
    pub divergence_status: String,
    pub commit_timestamp: Option<i64>,
    pub is_gone: bool,
    pub last_tool_usage: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MaterializeWorktreeResult {
    pub worktree: crate::commands::cleanup::WorktreeInfo,
    pub created: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BranchInventoryResolutionAction {
    FocusExisting,
    CreateWorktree,
    ResolveAmbiguity,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInventorySnapshotEntry {
    pub id: String,
    pub canonical_name: String,
    pub primary_branch: BranchInfo,
    pub local_branch: Option<BranchInfo>,
    pub remote_branch: Option<BranchInfo>,
    pub has_local: bool,
    pub has_remote: bool,
    pub worktree_path: Option<String>,
    pub worktree_count: usize,
    pub resolution_action: BranchInventoryResolutionAction,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchInventoryDetail {
    pub id: String,
    pub canonical_name: String,
    pub primary_branch: BranchInfo,
    pub local_branch: Option<BranchInfo>,
    pub remote_branch: Option<BranchInfo>,
    pub has_local: bool,
    pub has_remote: bool,
    pub worktree_path: Option<String>,
    pub worktree_count: usize,
    pub resolution_action: BranchInventoryResolutionAction,
}

#[derive(Debug, Clone)]
struct BranchInventoryWorktree {
    path: String,
    branch: String,
}

impl From<BranchInventorySnapshotEntry> for BranchInventoryDetail {
    fn from(value: BranchInventorySnapshotEntry) -> Self {
        Self {
            id: value.id,
            canonical_name: value.canonical_name,
            primary_branch: value.primary_branch,
            local_branch: value.local_branch,
            remote_branch: value.remote_branch,
            has_local: value.has_local,
            has_remote: value.has_remote,
            worktree_path: value.worktree_path,
            worktree_count: value.worktree_count,
            resolution_action: value.resolution_action,
        }
    }
}

impl From<Branch> for BranchInfo {
    fn from(b: Branch) -> Self {
        let divergence_status = b.divergence_status().to_string();
        BranchInfo {
            name: b.name,
            display_name: None,
            commit: b.commit,
            is_current: b.is_current,
            is_agent_running: false,
            agent_status: "unknown".to_string(),
            has_remote: b.has_remote,
            upstream: b.upstream,
            ahead: b.ahead,
            behind: b.behind,
            divergence_status,
            commit_timestamp: b.commit_timestamp,
            is_gone: b.is_gone,
            last_tool_usage: None,
        }
    }
}

/// Per-branch metadata extracted from session files.
#[derive(Debug, Clone)]
struct SessionBranchMeta {
    agent_status: AgentStatus,
    display_name: Option<String>,
}

/// Build a map of branch name → SessionBranchMeta from session files.
/// For agents without Hook support, infers status from pane output.
fn build_session_branch_meta_map(
    repo_path: &Path,
    state: &AppState,
) -> HashMap<String, SessionBranchMeta> {
    let manager = match WorktreeManager::new(repo_path) {
        Ok(m) => m,
        Err(_) => return HashMap::new(),
    };
    let worktrees = match manager.list_basic() {
        Ok(wts) => wts,
        Err(_) => return HashMap::new(),
    };

    // Build branch → pane_id mapping for running panes
    let pane_map = build_branch_pane_map(state, repo_path);

    let mut map = HashMap::new();
    for wt in &worktrees {
        if let Some(branch_name) = &wt.branch {
            if let Some(mut session) = Session::load_for_worktree(&wt.path) {
                session.check_idle_timeout();

                let agent_status = if agent_has_hook_support(session.agent.as_deref()) {
                    // Claude Code: trust session file status
                    session.status
                } else if let Some(pane_id) = pane_map.get(branch_name) {
                    // Non-hook agent with running pane: infer from output
                    infer_status_from_pane(state, pane_id)
                } else {
                    // No running pane: use session status as-is
                    session.status
                };

                map.insert(
                    branch_name.clone(),
                    SessionBranchMeta {
                        agent_status,
                        display_name: session.display_name,
                    },
                );
            }
        }
    }
    map
}

/// Build a map of branch name → pane_id in the given repo, preferring running panes.
fn build_branch_pane_map(state: &AppState, repo_path: &Path) -> HashMap<String, String> {
    let panes_info: Vec<(String, String, bool)> = match state.pane_manager.lock() {
        Ok(manager) => manager
            .panes()
            .iter()
            .map(|pane| {
                (
                    pane.pane_id().to_string(),
                    pane.branch_name().to_string(),
                    matches!(pane.status(), PaneStatus::Running),
                )
            })
            .collect(),
        Err(_) => return HashMap::new(),
    };

    let launch_meta = match state.pane_launch_meta.lock() {
        Ok(meta) => meta,
        Err(_) => {
            // Fallback: use all panes without repo filtering.
            return select_preferred_branch_panes(panes_info);
        }
    };

    select_preferred_branch_panes(panes_info.into_iter().filter(|(pane_id, _, _)| {
        launch_meta
            .get(pane_id)
            .map(|meta| meta.repo_path.as_path() == repo_path)
            .unwrap_or(false)
    }))
}

fn select_preferred_branch_panes<I>(panes: I) -> HashMap<String, String>
where
    I: IntoIterator<Item = (String, String, bool)>,
{
    let mut preferred: HashMap<String, (String, bool)> = HashMap::new();
    for (pane_id, branch, is_running) in panes {
        match preferred.get_mut(&branch) {
            Some((selected_pane_id, selected_is_running)) => {
                if !*selected_is_running && is_running {
                    *selected_pane_id = pane_id;
                    *selected_is_running = true;
                }
            }
            None => {
                preferred.insert(branch, (pane_id, is_running));
            }
        }
    }

    preferred
        .into_iter()
        .map(|(branch, (pane_id, _))| (branch, pane_id))
        .collect()
}

/// Infer agent status from a pane's scrollback tail.
fn infer_status_from_pane(state: &AppState, pane_id: &str) -> AgentStatus {
    let process_alive = match state.pane_manager.lock() {
        Ok(manager) => manager
            .panes()
            .iter()
            .find(|p| p.pane_id() == pane_id)
            .map(|p| matches!(p.status(), PaneStatus::Running))
            .unwrap_or(false),
        Err(_) => false,
    };

    let scrollback_tail =
        capture_scrollback_tail_from_state(state, pane_id, 4096, None).unwrap_or_default();

    infer_agent_status(&scrollback_tail, process_alive)
}

fn agent_status_to_string(status: AgentStatus) -> String {
    match status {
        AgentStatus::Unknown => "unknown".to_string(),
        AgentStatus::Running => "running".to_string(),
        AgentStatus::WaitingInput => "waiting_input".to_string(),
        AgentStatus::Stopped => "stopped".to_string(),
    }
}

fn strip_known_remote_prefix<'a>(branch: &'a str, remotes: &[Remote]) -> &'a str {
    let Some((first, rest)) = branch.split_once('/') else {
        return branch;
    };
    if remotes.iter().any(|r| r.name == first) {
        return rest;
    }
    branch
}

fn branch_inventory_key(branch: &str, remotes: &[Remote]) -> String {
    strip_known_remote_prefix(branch, remotes)
        .trim()
        .to_string()
}

fn build_branch_inventory_snapshot_entries(
    local: Vec<BranchInfo>,
    remote: Vec<BranchInfo>,
    worktrees: Vec<BranchInventoryWorktree>,
    remotes: &[Remote],
) -> Vec<BranchInventorySnapshotEntry> {
    let mut local_by_key = HashMap::new();
    let mut remote_by_key = HashMap::new();
    let mut keys = HashSet::new();

    for info in local {
        let key = branch_inventory_key(&info.name, remotes);
        keys.insert(key.clone());
        local_by_key.insert(key, info);
    }

    for info in remote {
        let key = branch_inventory_key(&info.name, remotes);
        keys.insert(key.clone());
        remote_by_key.insert(key, info);
    }

    let mut worktrees_by_key: HashMap<String, Vec<BranchInventoryWorktree>> = HashMap::new();
    for worktree in worktrees {
        let key = branch_inventory_key(&worktree.branch, remotes);
        worktrees_by_key.entry(key).or_default().push(worktree);
    }

    let mut sorted_keys = keys.into_iter().collect::<Vec<_>>();
    sorted_keys.sort();

    sorted_keys
        .into_iter()
        .filter_map(|key| {
            let local_branch = local_by_key.remove(&key);
            let remote_branch = remote_by_key.remove(&key);
            let primary_branch = local_branch.clone().or_else(|| remote_branch.clone())?;
            let matching_worktrees = worktrees_by_key.remove(&key).unwrap_or_default();
            let worktree_count = matching_worktrees.len();
            let worktree_path = if worktree_count == 1 {
                matching_worktrees.into_iter().next().map(|worktree| worktree.path)
            } else {
                None
            };
            let resolution_action = match worktree_count {
                0 => BranchInventoryResolutionAction::CreateWorktree,
                1 => BranchInventoryResolutionAction::FocusExisting,
                _ => BranchInventoryResolutionAction::ResolveAmbiguity,
            };
            Some(BranchInventorySnapshotEntry {
                id: key.clone(),
                canonical_name: key,
                primary_branch,
                has_local: local_branch.is_some(),
                has_remote: remote_branch.is_some(),
                local_branch,
                remote_branch,
                worktree_path,
                worktree_count,
                resolution_action,
            })
        })
        .collect()
}

fn materialize_worktree_ref_impl(
    project_path: &str,
    branch_ref: &str,
    state: &AppState,
) -> Result<MaterializeWorktreeResult, String> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let manager = WorktreeManager::new(&repo_path).map_err(|e| e.to_string())?;
    let remotes = Remote::list(&repo_path).unwrap_or_default();
    let normalized_branch = strip_known_remote_prefix(branch_ref, &remotes).to_string();

    let mut existing = crate::commands::cleanup::list_worktrees_impl(project_path, state)?
        .into_iter()
        .filter(|info| info.branch == normalized_branch || info.branch == branch_ref)
        .collect::<Vec<_>>();

    if existing.len() > 1 {
        return Err(format!(
            "Multiple worktrees already exist for branch '{}'; resolve the ambiguity before focusing.",
            normalized_branch
        ));
    }

    if let Some(worktree) = existing.pop() {
        return Ok(MaterializeWorktreeResult {
            worktree,
            created: false,
        });
    }

    let created = manager
        .create_for_branch(branch_ref)
        .map_err(|e| e.to_string())?;

    let worktree = crate::commands::cleanup::list_worktrees_impl(project_path, state)?
        .into_iter()
        .find(|info| info.path == created.path.to_string_lossy())
        .ok_or_else(|| {
            format!(
                "Worktree was created for '{}' but could not be resolved in the refreshed listing.",
                branch_ref
            )
        })?;

    Ok(MaterializeWorktreeResult {
        worktree,
        created: true,
    })
}

fn build_last_tool_usage_map(repo_path: &Path) -> HashMap<String, String> {
    gwt_core::config::get_last_tool_usage_map(repo_path)
        .into_iter()
        .map(|(branch, entry)| (branch, entry.format_tool_usage()))
        .collect()
}

fn running_agent_branches(state: &AppState, repo_path: &Path) -> HashSet<String> {
    let running: Vec<(String, String)> = match state.pane_manager.lock() {
        Ok(manager) => manager
            .panes()
            .iter()
            .filter(|pane| matches!(pane.status(), gwt_core::terminal::pane::PaneStatus::Running))
            .map(|pane| (pane.pane_id().to_string(), pane.branch_name().to_string()))
            .collect(),
        Err(_) => Vec::new(),
    };

    if running.is_empty() {
        return HashSet::new();
    }

    let Ok(launch_meta) = state.pane_launch_meta.lock() else {
        return running.into_iter().map(|(_, branch)| branch).collect();
    };

    running
        .into_iter()
        .filter_map(|(pane_id, branch)| {
            let meta = launch_meta.get(&pane_id)?;
            if meta.repo_path.as_path() == repo_path {
                Some(branch)
            } else {
                None
            }
        })
        .collect()
}

fn with_panic_guard<T>(
    context: &str,
    command: &str,
    f: impl FnOnce() -> Result<T, StructuredError>,
) -> Result<T, StructuredError> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(result) => result,
        Err(_) => {
            error!(
                category = "tauri",
                operation = context,
                "Unexpected panic while handling branch command"
            );
            Err(StructuredError::internal(
                &format!("Unexpected error while {}", context),
                command,
            ))
        }
    }
}

#[derive(Debug)]
struct WorktreeBranchListing {
    infos: Vec<BranchInfo>,
    branch_names: Vec<String>,
}

fn is_unknown_display_name(text: &str) -> bool {
    matches!(
        text.trim(),
        "" | "Unknown" | "(Unknown)" | "Not available" | "(Not available)" | "不明" | "(不明)"
    )
}

fn extract_issue_number(branch_name: &str) -> Option<u64> {
    let normalized = branch_name.trim().trim_start_matches("origin/");
    for part in normalized.split('/') {
        let lower = part.to_ascii_lowercase();
        let Some(rest) = lower.strip_prefix("issue-") else {
            continue;
        };
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(number) = digits.parse::<u64>() {
            return Some(number);
        }
    }
    None
}

fn extract_issue_label(branch_name: &str) -> Option<String> {
    extract_issue_number(branch_name).map(|number| format!("#{number}"))
}

fn is_raw_branch_preserved(branch_name: &str) -> bool {
    matches!(
        branch_name.trim().trim_start_matches("origin/"),
        "main" | "master" | "develop"
    )
}

fn format_issue_display_name(number: u64, title: &str) -> Option<String> {
    let title = title.trim();
    if title.is_empty() {
        None
    } else {
        Some(format!("#{number} {title}"))
    }
}

fn build_issue_display_name_map_with<I, F>(
    branch_names: I,
    fetch_issue: F,
) -> HashMap<String, String>
where
    I: IntoIterator,
    I::Item: AsRef<str>,
    F: Fn(u64) -> Result<(u64, String), String>,
{
    let mut resolved_by_issue = HashMap::<u64, Option<String>>::new();
    let mut display_names = HashMap::<String, String>::new();

    for branch_name in branch_names {
        let branch_name = branch_name.as_ref().trim();
        if branch_name.is_empty() || is_raw_branch_preserved(branch_name) {
            continue;
        }

        let Some(issue_number) = extract_issue_number(branch_name) else {
            continue;
        };

        let resolved = if let Some(existing) = resolved_by_issue.get(&issue_number) {
            existing.clone()
        } else {
            let next = fetch_issue(issue_number)
                .ok()
                .and_then(|(number, title)| format_issue_display_name(number, &title));
            resolved_by_issue.insert(issue_number, next.clone());
            next
        };

        if let Some(display_name) = resolved {
            display_names.insert(branch_name.to_string(), display_name);
        }
    }

    display_names
}

fn build_cached_issue_title_map(state: &AppState, repo_path: &Path) -> HashMap<u64, String> {
    let repo_key = repo_path.to_string_lossy().to_string();
    let Ok(guard) = state.project_issue_list_cache.lock() else {
        return HashMap::new();
    };
    let Some(entries) = guard.get(&repo_key) else {
        return HashMap::new();
    };

    let mut titles = HashMap::<u64, String>::new();
    for entry in entries.values() {
        let Ok(response) = serde_json::from_str::<FetchIssuesResponse>(&entry.response_json) else {
            continue;
        };
        for issue in response.issues {
            titles.entry(issue.number).or_insert(issue.title);
        }
    }

    titles
}

fn build_issue_display_name_map(
    branch_names: &[String],
    repo_path: &Path,
    state: &AppState,
) -> HashMap<String, String> {
    let cached_titles = build_cached_issue_title_map(state, repo_path);
    build_issue_display_name_map_with(branch_names.iter(), |issue_number| {
        if let Some(title) = cached_titles.get(&issue_number).cloned() {
            return Ok((issue_number, title));
        }
        let issue = fetch_issue_detail(repo_path, issue_number)?;
        Ok((issue.number, issue.title))
    })
}

fn branch_inventory_detail_names(
    entry: &BranchInventorySnapshotEntry,
    remotes: &[Remote],
) -> Vec<String> {
    let mut names = HashSet::new();
    names.insert(branch_inventory_key(&entry.primary_branch.name, remotes));
    if let Some(local_branch) = &entry.local_branch {
        names.insert(branch_inventory_key(&local_branch.name, remotes));
    }
    if let Some(remote_branch) = &entry.remote_branch {
        names.insert(branch_inventory_key(&remote_branch.name, remotes));
    }
    names.into_iter().collect()
}

fn apply_branch_inventory_detail(
    info: &mut BranchInfo,
    remotes: &[Remote],
    last_tool: &HashMap<String, String>,
    running_branches: &HashSet<String>,
    meta_map: &HashMap<String, SessionBranchMeta>,
    summary_cache: &Option<&gwt_core::ai::SessionSummaryCache>,
    issue_display_names: &HashMap<String, String>,
) {
    let branch_key = branch_inventory_key(&info.name, remotes);
    info.last_tool_usage = last_tool.get(&branch_key).cloned();
    info.is_agent_running = running_branches.contains(&branch_key);
    apply_session_meta(info, &branch_key, meta_map, summary_cache, issue_display_names);
}

fn build_branch_inventory_detail(
    snapshot: BranchInventorySnapshotEntry,
    repo_path: &Path,
    state: &AppState,
    remotes: &[Remote],
) -> BranchInventoryDetail {
    let last_tool = build_last_tool_usage_map(repo_path);
    let running_branches = running_agent_branches(state, repo_path);
    let meta_map = build_session_branch_meta_map(repo_path, state);
    let repo_key = repo_path.to_string_lossy().to_string();
    let summary_cache_guard = state.session_summary_cache.lock().ok();
    let summary_cache = summary_cache_guard.as_ref().and_then(|g| g.get(&repo_key));
    let branch_names = branch_inventory_detail_names(&snapshot, remotes);
    let issue_display_names = build_issue_display_name_map(&branch_names, repo_path, state);

    let mut detail = BranchInventoryDetail::from(snapshot);
    apply_branch_inventory_detail(
        &mut detail.primary_branch,
        remotes,
        &last_tool,
        &running_branches,
        &meta_map,
        &summary_cache,
        &issue_display_names,
    );
    if let Some(local_branch) = detail.local_branch.as_mut() {
        apply_branch_inventory_detail(
            local_branch,
            remotes,
            &last_tool,
            &running_branches,
            &meta_map,
            &summary_cache,
            &issue_display_names,
        );
    }
    if let Some(remote_branch) = detail.remote_branch.as_mut() {
        apply_branch_inventory_detail(
            remote_branch,
            remotes,
            &last_tool,
            &running_branches,
            &meta_map,
            &summary_cache,
            &issue_display_names,
        );
    }
    detail
}

fn try_get_branch_inventory_snapshot_cache(
    state: &AppState,
    repo_key: &str,
    refresh_key: u64,
) -> Option<Vec<BranchInventorySnapshotEntry>> {
    let guard = state.project_branch_inventory_snapshot_cache.lock().ok()?;
    let entry = guard.get(repo_key)?;
    if entry.refresh_key != refresh_key {
        return None;
    }
    Some(entry.entries.clone())
}

fn put_branch_inventory_snapshot_cache(
    state: &AppState,
    repo_key: &str,
    refresh_key: u64,
    entries: &[BranchInventorySnapshotEntry],
) {
    if let Ok(mut guard) = state.project_branch_inventory_snapshot_cache.lock() {
        guard.insert(
            repo_key.to_string(),
            crate::state::BranchInventorySnapshotCacheEntry {
                refresh_key,
                entries: entries.to_vec(),
            },
        );
    }
    if let Ok(mut guard) = state.project_branch_inventory_detail_cache.lock() {
        guard.remove(repo_key);
    }
}

fn mark_branch_inventory_snapshot_inflight(state: &AppState, inflight_key: &str) -> bool {
    if let Ok(mut set) = state.project_branch_inventory_snapshot_inflight.lock() {
        if set.contains(inflight_key) {
            return false;
        }
        set.insert(inflight_key.to_string());
        return true;
    }
    true
}

fn clear_branch_inventory_snapshot_inflight(state: &AppState, inflight_key: &str) {
    if let Ok(mut set) = state.project_branch_inventory_snapshot_inflight.lock() {
        set.remove(inflight_key);
    }
}

fn wait_for_branch_inventory_snapshot_inflight(state: &AppState, inflight_key: &str) {
    let mut waited_ms: u64 = 0;
    while waited_ms < BRANCH_INVENTORY_INFLIGHT_WAIT_MS {
        let still_inflight = state
            .project_branch_inventory_snapshot_inflight
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

fn try_get_branch_inventory_detail_cache(
    state: &AppState,
    repo_key: &str,
    canonical_name: &str,
) -> Option<BranchInventoryDetail> {
    let guard = state.project_branch_inventory_detail_cache.lock().ok()?;
    let repo_entries = guard.get(repo_key)?;
    repo_entries.get(canonical_name).map(|entry| entry.detail.clone())
}

fn put_branch_inventory_detail_cache(
    state: &AppState,
    repo_key: &str,
    canonical_name: &str,
    detail: &BranchInventoryDetail,
) {
    if let Ok(mut guard) = state.project_branch_inventory_detail_cache.lock() {
        let repo_entries = guard.entry(repo_key.to_string()).or_default();
        repo_entries.insert(
            canonical_name.to_string(),
            crate::state::BranchInventoryDetailCacheEntry {
                detail: detail.clone(),
            },
        );
    }
}

fn branch_topic_label(branch_name: &str) -> Option<String> {
    if let Some(issue) = extract_issue_label(branch_name) {
        return Some(issue);
    }

    let normalized = branch_name.trim().trim_start_matches("origin/");
    let topic = normalized
        .split('/')
        .next_back()
        .unwrap_or(normalized)
        .trim();
    if topic.is_empty() {
        return None;
    }

    let humanized = topic.replace(['-', '_'], " ");
    let normalized_spaces = humanized.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized_spaces.is_empty() {
        None
    } else {
        Some(normalized_spaces)
    }
}

fn strip_inferred_prefix(text: &str) -> &str {
    let trimmed = text.trim();
    if let Some(rest) = trimmed.strip_prefix("（推定）") {
        return rest.trim();
    }
    if let Some(rest) = trimmed.strip_prefix("(Inferred)") {
        return rest.trim();
    }
    trimmed
}

fn first_display_line(text: &str) -> &str {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("")
}

fn looks_like_code_fragment(text: &str) -> bool {
    let trimmed = text.trim();
    trimmed.starts_with("fn ")
        || trimmed.starts_with("pub ")
        || trimmed.starts_with("impl ")
        || trimmed.starts_with("let ")
        || trimmed.contains("->")
        || trimmed.contains("&str")
        || trimmed.contains("SummaryLanguage")
        || trimmed.contains("::")
        || trimmed.contains('{')
        || trimmed.contains('}')
}

fn normalize_generated_display_name(raw: &str, branch_name: &str) -> Option<String> {
    let mut candidate = strip_inferred_prefix(first_display_line(raw))
        .trim()
        .to_string();
    if candidate.is_empty() || is_unknown_display_name(&candidate) {
        return None;
    }
    if looks_like_code_fragment(&candidate) {
        return None;
    }

    if candidate.starts_with("Deliver the outcome intended by branch '")
        || candidate == "Deliver the primary outcome for this worktree"
        || candidate == "Advance this worktree outcome"
    {
        return branch_topic_label(branch_name);
    }

    for suffix in [
        " に関する成果をこのWorktreeで達成すること",
        "に関する成果をこのWorktreeで達成すること",
        " をこのWorktreeで達成すること",
        "をこのWorktreeで達成すること",
    ] {
        if let Some(prefix) = candidate.strip_suffix(suffix) {
            let trimmed = prefix.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
            return branch_topic_label(branch_name);
        }
    }

    if candidate == "このWorktreeで進めている成果を達成すること" {
        return branch_topic_label(branch_name);
    }

    if let Some(prefix) = candidate.strip_suffix("こと") {
        let trimmed = prefix.trim();
        if !trimmed.is_empty() {
            candidate = trimmed.to_string();
        }
    }

    if is_unknown_display_name(&candidate) {
        return None;
    }

    Some(candidate)
}

fn resolve_auto_display_name(
    branch_key: &str,
    issue_display_names: &HashMap<String, String>,
    task_overview: Option<&str>,
) -> Option<String> {
    if is_raw_branch_preserved(branch_key) {
        return None;
    }
    if let Some(display_name) = issue_display_names.get(branch_key) {
        return Some(display_name.clone());
    }
    task_overview.and_then(|overview| normalize_generated_display_name(overview, branch_key))
}

/// Apply session branch meta (agent_status + display_name) to a BranchInfo.
/// `branch_key` is the lookup key in the meta map (may differ from info.name for remote branches).
fn apply_session_meta(
    info: &mut BranchInfo,
    branch_key: &str,
    meta_map: &HashMap<String, SessionBranchMeta>,
    summary_cache: &Option<&gwt_core::ai::SessionSummaryCache>,
    issue_display_names: &HashMap<String, String>,
) {
    if let Some(meta) = meta_map.get(branch_key) {
        info.agent_status = agent_status_to_string(meta.agent_status);
        // display_name priority: session.display_name → linked issue → task_overview
        if meta.display_name.is_some() {
            info.display_name = meta.display_name.clone();
        }
    }
    if info.display_name.is_none() {
        let task_overview = summary_cache
            .and_then(|cache| cache.get(branch_key))
            .and_then(|summary| summary.task_overview.as_deref());
        if let Some(display_name) =
            resolve_auto_display_name(branch_key, issue_display_names, task_overview)
        {
            info.display_name = Some(display_name);
        }
    }
}

fn list_worktree_branches_impl(
    project_path: &str,
    state: &AppState,
) -> Result<WorktreeBranchListing, StructuredError> {
    let _span = tracing::info_span!(
        "startup.list_worktree_branches_impl",
        project_path = %project_path
    )
    .entered();
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_worktree_branches"))?;
    let last_tool = build_last_tool_usage_map(&repo_path);
    let running_branches = running_agent_branches(state, &repo_path);
    let meta_map = build_session_branch_meta_map(&repo_path, state);
    let repo_key = repo_path.to_string_lossy().to_string();
    let summary_cache_guard = state.session_summary_cache.lock().ok();
    let summary_cache = summary_cache_guard.as_ref().and_then(|g| g.get(&repo_key));

    let manager = WorktreeManager::new(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_worktree_branches"))?;
    let worktrees = manager
        .list_basic()
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_worktree_branches"))?;

    let names: HashSet<String> = worktrees
        .into_iter()
        .filter(|wt| !wt.is_main && wt.is_active())
        .filter_map(|wt| wt.branch)
        .collect();

    if names.is_empty() {
        return Ok(WorktreeBranchListing {
            infos: Vec::new(),
            branch_names: Vec::new(),
        });
    }

    let branch_names = names.iter().cloned().collect::<Vec<_>>();
    let issue_display_names = build_issue_display_name_map(&branch_names, &repo_path, state);

    let branches = Branch::list(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_worktree_branches"))?;
    let mut infos: Vec<BranchInfo> = branches
        .into_iter()
        .filter(|b| names.contains(&b.name))
        .map(BranchInfo::from)
        .collect();
    for info in &mut infos {
        info.last_tool_usage = last_tool.get(&info.name).cloned();
        info.is_agent_running = running_branches.contains(&info.name);
        apply_session_meta(
            info,
            &info.name.clone(),
            &meta_map,
            &summary_cache,
            &issue_display_names,
        );
    }

    Ok(WorktreeBranchListing {
        infos,
        branch_names,
    })
}

fn list_remote_branches_impl(
    project_path: &str,
    state: &AppState,
) -> Result<Vec<BranchInfo>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_remote_branches"))?;
    let last_tool = build_last_tool_usage_map(&repo_path);
    let running_branches = running_agent_branches(state, &repo_path);
    let meta_map = build_session_branch_meta_map(&repo_path, state);
    let repo_key = repo_path.to_string_lossy().to_string();
    let summary_cache_guard = state.session_summary_cache.lock().ok();
    let summary_cache = summary_cache_guard.as_ref().and_then(|g| g.get(&repo_key));
    let remotes = Remote::list(&repo_path).unwrap_or_default();

    let branches = if is_bare_repository(&repo_path) {
        Branch::list_remote_from_origin(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "list_remote_branches"))?
    } else {
        Branch::list_remote(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "list_remote_branches"))?
    };
    let normalized_branch_names = branches
        .iter()
        .map(|branch| strip_known_remote_prefix(&branch.name, &remotes).to_string())
        .collect::<Vec<_>>();
    let issue_display_names =
        build_issue_display_name_map(&normalized_branch_names, &repo_path, state);

    let mut infos: Vec<BranchInfo> = branches.into_iter().map(BranchInfo::from).collect();
    for info in &mut infos {
        let normalized = strip_known_remote_prefix(&info.name, &remotes).to_string();
        info.last_tool_usage = last_tool.get(&normalized).cloned();
        info.is_agent_running = running_branches.contains(&normalized);
        apply_session_meta(
            info,
            &normalized,
            &meta_map,
            &summary_cache,
            &issue_display_names,
        );
    }

    Ok(infos)
}

fn list_local_inventory_branches_impl(project_path: &str) -> Result<Vec<BranchInfo>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_branch_inventory"))?;
    let branches = Branch::list(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_branch_inventory"))?;
    Ok(branches.into_iter().map(BranchInfo::from).collect())
}

fn list_remote_inventory_branches_impl(
    project_path: &str,
) -> Result<Vec<BranchInfo>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_branch_inventory"))?;
    let branches = if is_bare_repository(&repo_path) {
        Branch::list_remote_from_origin(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "list_branch_inventory"))?
    } else {
        Branch::list_remote(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "list_branch_inventory"))?
    };
    Ok(branches.into_iter().map(BranchInfo::from).collect())
}

fn list_branch_inventory_worktrees_impl(
    project_path: &str,
) -> Result<Vec<BranchInventoryWorktree>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_branch_inventory"))?;
    let manager = WorktreeManager::new(&repo_path)
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_branch_inventory"))?;
    let worktrees = manager
        .list_basic()
        .map_err(|e| StructuredError::from_gwt_error(&e, "list_branch_inventory"))?;
    Ok(worktrees
        .into_iter()
        .filter(|worktree| !worktree.is_main && worktree.is_active())
        .filter_map(|worktree| {
            let branch = worktree.branch?;
            Some(BranchInventoryWorktree {
                path: worktree.path.to_string_lossy().to_string(),
                branch,
            })
        })
        .collect())
}

fn list_branch_inventory_impl(
    project_path: &str,
    refresh_key: u64,
    state: &AppState,
) -> Result<Vec<BranchInventorySnapshotEntry>, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "list_branch_inventory"))?;
    let repo_key = repo_path.to_string_lossy().to_string();
    if let Some(hit) = try_get_branch_inventory_snapshot_cache(state, &repo_key, refresh_key) {
        return Ok(hit);
    }
    let inflight_key = format!("{repo_key}::{refresh_key}");
    let fetch_owner = mark_branch_inventory_snapshot_inflight(state, &inflight_key);
    if !fetch_owner {
        wait_for_branch_inventory_snapshot_inflight(state, &inflight_key);
        if let Some(hit) = try_get_branch_inventory_snapshot_cache(state, &repo_key, refresh_key) {
            return Ok(hit);
        }
    }

    let result = (|| {
        let remotes = Remote::list(&repo_path).unwrap_or_default();
        let local = list_local_inventory_branches_impl(project_path)?;
        let remote = list_remote_inventory_branches_impl(project_path)?;
        let worktrees = list_branch_inventory_worktrees_impl(project_path)?;
        let entries = build_branch_inventory_snapshot_entries(local, remote, worktrees, &remotes);
        put_branch_inventory_snapshot_cache(state, &repo_key, refresh_key, &entries);
        Ok(entries)
    })();

    if fetch_owner {
        clear_branch_inventory_snapshot_inflight(state, &inflight_key);
    }

    result
}

fn get_branch_inventory_detail_impl(
    project_path: &str,
    canonical_name: &str,
    force_refresh: bool,
    state: &AppState,
) -> Result<BranchInventoryDetail, StructuredError> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)
        .map_err(|e| StructuredError::internal(&e, "get_branch_inventory_detail"))?;
    let repo_key = repo_path.to_string_lossy().to_string();
    let remotes = Remote::list(&repo_path).unwrap_or_default();
    let canonical_name = branch_inventory_key(canonical_name, &remotes);

    if !force_refresh {
        if let Some(hit) = try_get_branch_inventory_detail_cache(state, &repo_key, &canonical_name)
        {
            return Ok(hit);
        }
    }

    let refresh_key = state
        .project_branch_inventory_snapshot_cache
        .lock()
        .ok()
        .and_then(|guard| guard.get(&repo_key).map(|entry| entry.refresh_key))
        .unwrap_or(0);
    let snapshots = list_branch_inventory_impl(project_path, refresh_key, state)?;
    let snapshot = snapshots
        .into_iter()
        .find(|entry| entry.canonical_name == canonical_name)
        .ok_or_else(|| {
            StructuredError::internal(
                &format!("Branch inventory detail not found for '{}'", canonical_name),
                "get_branch_inventory_detail",
            )
        })?;
    let detail = build_branch_inventory_detail(snapshot, &repo_path, state, &remotes);
    put_branch_inventory_detail_cache(state, &repo_key, &canonical_name, &detail);
    Ok(detail)
}

/// List all local branches in a repository
#[instrument(skip_all, fields(command = "list_branches", project_path))]
#[tauri::command]
pub fn list_branches(
    project_path: String,
    state: State<AppState>,
) -> Result<Vec<BranchInfo>, StructuredError> {
    with_panic_guard("listing branches", "list_branches", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)
            .map_err(|e| StructuredError::internal(&e, "list_branches"))?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);
        let meta_map = build_session_branch_meta_map(&repo_path, &state);
        let repo_key = repo_path.to_string_lossy().to_string();
        let summary_cache_guard = state.session_summary_cache.lock().ok();
        let summary_cache = summary_cache_guard.as_ref().and_then(|g| g.get(&repo_key));

        let branches = Branch::list(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "list_branches"))?;
        let issue_branch_names = branches
            .iter()
            .map(|branch| branch.name.clone())
            .collect::<Vec<_>>();
        let issue_display_names =
            build_issue_display_name_map(&issue_branch_names, &repo_path, &state);
        let mut infos: Vec<BranchInfo> = branches.into_iter().map(BranchInfo::from).collect();
        for info in &mut infos {
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
            apply_session_meta(
                info,
                &info.name.clone(),
                &meta_map,
                &summary_cache,
                &issue_display_names,
            );
        }
        Ok(infos)
    })
}

#[instrument(skip_all, fields(command = "list_branch_inventory", project_path))]
#[tauri::command]
pub async fn list_branch_inventory(
    project_path: String,
    refresh_key: Option<u64>,
    app_handle: AppHandle,
) -> Result<Vec<BranchInventorySnapshotEntry>, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        with_panic_guard("listing branch inventory", "list_branch_inventory", || {
            list_branch_inventory_impl(&project_path, refresh_key.unwrap_or(0), &state)
        })
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Unexpected error while listing branch inventory: {e}"),
            "list_branch_inventory",
        )
    })?
}

#[instrument(skip_all, fields(command = "get_branch_inventory_detail", project_path, canonical_name))]
#[tauri::command]
pub async fn get_branch_inventory_detail(
    project_path: String,
    canonical_name: String,
    force_refresh: Option<bool>,
    app_handle: AppHandle,
) -> Result<BranchInventoryDetail, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle.state::<AppState>();
        with_panic_guard(
            "getting branch inventory detail",
            "get_branch_inventory_detail",
            || {
                get_branch_inventory_detail_impl(
                    &project_path,
                    &canonical_name,
                    force_refresh.unwrap_or(false),
                    &state,
                )
            },
        )
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Unexpected error while getting branch inventory detail: {e}"),
            "get_branch_inventory_detail",
        )
    })?
}

/// List branches that currently have a local worktree (gwt "Local" view)
#[instrument(skip_all, fields(command = "list_worktree_branches", project_path))]
#[tauri::command]
pub async fn list_worktree_branches(
    project_path: String,
    app_handle: AppHandle,
) -> Result<Vec<BranchInfo>, StructuredError> {
    let started = Instant::now();
    let project_path_for_warn = project_path.clone();
    tauri::async_runtime::spawn_blocking(move || {
        with_panic_guard(
            "listing worktree branches",
            "list_worktree_branches",
            || {
                let state = app_handle.state::<AppState>();
                let listing = list_worktree_branches_impl(&project_path, &state)?;

                let prewarm_project_path = project_path.clone();
                let prewarm_handle = app_handle.clone();
                let branch_names = listing.branch_names;
                tauri::async_runtime::spawn_blocking(move || {
                    crate::commands::sessions::prewarm_missing_worktree_summaries(
                        prewarm_project_path,
                        branch_names,
                        prewarm_handle,
                    );
                });

                Ok(listing.infos)
            },
        )
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Unexpected error while listing worktree branches: {e}"),
            "list_worktree_branches",
        )
    })
    .inspect(|_result| {
        let elapsed = started.elapsed();
        if elapsed > LIST_WORKTREE_BRANCHES_WARN_THRESHOLD {
            warn!(
                category = "project_start",
                command = "list_worktree_branches",
                project_path = %project_path_for_warn,
                elapsed_ms = elapsed.as_millis(),
                "list_worktree_branches took longer than expected"
            );
        }
    })?
}

/// List all remote branches in a repository
#[instrument(skip_all, fields(command = "list_remote_branches", project_path))]
#[tauri::command]
pub async fn list_remote_branches(
    project_path: String,
    app_handle: AppHandle,
) -> Result<Vec<BranchInfo>, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        with_panic_guard("listing remote branches", "list_remote_branches", || {
            let state = app_handle.state::<AppState>();
            list_remote_branches_impl(&project_path, &state)
        })
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Unexpected error while listing remote branches: {e}"),
            "list_remote_branches",
        )
    })?
}

#[instrument(
    skip_all,
    fields(command = "materialize_worktree_ref", project_path, branch_ref)
)]
#[tauri::command]
pub async fn materialize_worktree_ref(
    project_path: String,
    branch_ref: String,
    app_handle: AppHandle,
) -> Result<MaterializeWorktreeResult, StructuredError> {
    tauri::async_runtime::spawn_blocking(move || {
        with_panic_guard(
            "materializing worktree ref",
            "materialize_worktree_ref",
            || {
                let state = app_handle.state::<AppState>();
                materialize_worktree_ref_impl(&project_path, &branch_ref, &state)
                    .map_err(|e| StructuredError::internal(&e, "materialize_worktree_ref"))
            },
        )
    })
    .await
    .map_err(|e| {
        StructuredError::internal(
            &format!("Unexpected error while materializing worktree ref: {e}"),
            "materialize_worktree_ref",
        )
    })?
}

/// Get the current branch
#[instrument(skip_all, fields(command = "get_current_branch", project_path))]
#[tauri::command]
pub fn get_current_branch(
    project_path: String,
    state: State<AppState>,
) -> Result<Option<BranchInfo>, StructuredError> {
    with_panic_guard("getting current branch", "get_current_branch", || {
        let project_root = Path::new(&project_path);
        let repo_path = resolve_repo_path_for_project_root(project_root)
            .map_err(|e| StructuredError::internal(&e, "get_current_branch"))?;
        let branch = Branch::current(&repo_path)
            .map_err(|e| StructuredError::from_gwt_error(&e, "get_current_branch"))?;
        let last_tool = build_last_tool_usage_map(&repo_path);
        let running_branches = running_agent_branches(&state, &repo_path);
        let meta_map = build_session_branch_meta_map(&repo_path, &state);
        let repo_key = repo_path.to_string_lossy().to_string();
        let summary_cache_guard = state.session_summary_cache.lock().ok();
        let summary_cache = summary_cache_guard.as_ref().and_then(|g| g.get(&repo_key));
        let issue_display_names = branch
            .as_ref()
            .map(|branch| {
                build_issue_display_name_map(std::slice::from_ref(&branch.name), &repo_path, &state)
            })
            .unwrap_or_default();
        Ok(branch.map(|b| {
            let mut info = BranchInfo::from(b);
            info.last_tool_usage = last_tool.get(&info.name).cloned();
            info.is_agent_running = running_branches.contains(&info.name);
            let name_key = info.name.clone();
            apply_session_meta(
                &mut info,
                &name_key,
                &meta_map,
                &summary_cache,
                &issue_display_names,
            );
            info
        }))
    })
}

#[cfg(test)]
mod tests {
    use gwt_core::{config::AgentStatus, process::command};
    use tempfile::TempDir;

    use super::*;
    use crate::state::{AppState, IssueListCacheEntry};

    fn init_git_repo(path: &Path) {
        let init = command("git").args(["init"]).current_dir(path).output();
        assert!(init.is_ok(), "git init failed to run");
        assert!(init.unwrap().status.success(), "git init failed");

        let _ = command("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(path)
            .output();
        let _ = command("git")
            .args(["config", "user.name", "test"])
            .current_dir(path)
            .output();

        std::fs::write(path.join("README.md"), "init\n").expect("failed to write README");
        let add = command("git")
            .args(["add", "README.md"])
            .current_dir(path)
            .output()
            .expect("git add should run");
        assert!(add.status.success(), "git add failed");

        let commit = command("git")
            .args(["commit", "-m", "init"])
            .current_dir(path)
            .output()
            .expect("git commit should run");
        assert!(commit.status.success(), "git commit failed");
    }

    #[test]
    fn test_with_panic_guard_returns_error_on_panic() {
        let result: Result<(), StructuredError> =
            with_panic_guard("test", "test_cmd", || -> Result<(), StructuredError> {
                panic!("boom");
            });
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_status_to_string_unknown() {
        assert_eq!(agent_status_to_string(AgentStatus::Unknown), "unknown");
    }

    #[test]
    fn test_agent_status_to_string_running() {
        assert_eq!(agent_status_to_string(AgentStatus::Running), "running");
    }

    #[test]
    fn test_agent_status_to_string_waiting_input() {
        assert_eq!(
            agent_status_to_string(AgentStatus::WaitingInput),
            "waiting_input"
        );
    }

    #[test]
    fn test_agent_status_to_string_stopped() {
        assert_eq!(agent_status_to_string(AgentStatus::Stopped), "stopped");
    }

    #[test]
    fn test_branch_info_default_agent_status() {
        let branch = gwt_core::git::Branch {
            name: "feature/test".to_string(),
            commit: "abc1234".to_string(),
            is_current: false,
            has_remote: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
            is_gone: false,
        };
        let info = BranchInfo::from(branch);
        assert_eq!(info.agent_status, "unknown");
        assert!(!info.is_agent_running);
    }

    #[test]
    fn test_select_preferred_branch_panes_prefers_running_pane() {
        let panes = vec![
            ("pane-completed".to_string(), "feature/a".to_string(), false),
            ("pane-running".to_string(), "feature/a".to_string(), true),
        ];

        let map = select_preferred_branch_panes(panes);
        assert_eq!(
            map.get("feature/a").map(String::as_str),
            Some("pane-running")
        );
    }

    #[test]
    fn test_select_preferred_branch_panes_keeps_first_when_not_running() {
        let panes = vec![
            ("pane-old".to_string(), "feature/a".to_string(), false),
            ("pane-new".to_string(), "feature/a".to_string(), false),
        ];

        let map = select_preferred_branch_panes(panes);
        assert_eq!(map.get("feature/a").map(String::as_str), Some("pane-old"));
    }

    #[test]
    fn test_list_worktree_branches_impl_returns_consistent_branch_mapping() {
        let repo = TempDir::new().expect("temp dir");
        init_git_repo(repo.path());
        let project_path = repo.path().to_string_lossy().to_string();
        let state = AppState::new();

        let out = list_worktree_branches_impl(&project_path, &state).expect("listing should work");
        let names: HashSet<String> = out.branch_names.iter().cloned().collect();
        assert_eq!(names.len(), out.branch_names.len());
        for info in &out.infos {
            assert!(names.contains(&info.name));
        }
    }

    #[test]
    fn test_list_remote_branches_impl_returns_empty_without_remotes() {
        let repo = TempDir::new().expect("temp dir");
        init_git_repo(repo.path());
        let project_path = repo.path().to_string_lossy().to_string();
        let state = AppState::new();

        let out = list_remote_branches_impl(&project_path, &state).expect("listing should work");
        assert!(out.is_empty());
    }

    #[test]
    fn test_materialize_worktree_ref_impl_reuses_existing_worktree() {
        let repo = TempDir::new().expect("temp dir");
        init_git_repo(repo.path());
        let branch = "feature/browser-open";
        let create_branch = command("git")
            .args(["branch", branch])
            .current_dir(repo.path())
            .output()
            .expect("git branch should run");
        assert!(create_branch.status.success(), "git branch failed");

        let project_path = repo.path().to_string_lossy().to_string();
        let state = AppState::new();

        let first =
            materialize_worktree_ref_impl(&project_path, branch, &state).expect("first create");
        assert!(first.created);
        assert_eq!(first.worktree.branch, branch);

        let second =
            materialize_worktree_ref_impl(&project_path, branch, &state).expect("reuse existing");
        assert!(!second.created);
        assert_eq!(second.worktree.branch, branch);
        assert_eq!(second.worktree.path, first.worktree.path);
    }

    fn make_branch_info(name: &str) -> BranchInfo {
        BranchInfo {
            name: name.to_string(),
            display_name: None,
            commit: "abc1234".to_string(),
            is_current: false,
            is_agent_running: false,
            agent_status: "unknown".to_string(),
            has_remote: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            divergence_status: "UpToDate".to_string(),
            commit_timestamp: Some(1_700_000_000_000),
            is_gone: false,
            last_tool_usage: None,
        }
    }

    fn make_inventory_worktree(path: &str, branch: &str) -> BranchInventoryWorktree {
        BranchInventoryWorktree {
            path: path.to_string(),
            branch: branch.to_string(),
        }
    }

    #[test]
    fn test_build_branch_inventory_snapshot_entries_merges_local_and_remote_refs() {
        let entries = build_branch_inventory_snapshot_entries(
            vec![make_branch_info("feature/inventory")],
            vec![make_branch_info("origin/feature/inventory")],
            vec![make_inventory_worktree(
                "/tmp/wt-feature-inventory",
                "feature/inventory",
            )],
            &[Remote::new("origin", "https://example.com/repo.git")],
        );

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.canonical_name, "feature/inventory");
        assert!(entry.has_local);
        assert!(entry.has_remote);
        assert_eq!(entry.primary_branch.name, "feature/inventory");
        assert_eq!(
            entry.resolution_action,
            BranchInventoryResolutionAction::FocusExisting
        );
        assert_eq!(entry.worktree_count, 1);
        assert_eq!(entry.worktree_path.as_deref(), Some("/tmp/wt-feature-inventory"));
        assert_eq!(entry.primary_branch.display_name, None);
        assert_eq!(entry.primary_branch.last_tool_usage, None);
        assert_eq!(entry.primary_branch.agent_status, "unknown");
    }

    #[test]
    fn test_build_branch_inventory_snapshot_entries_marks_ambiguous_worktrees() {
        let entries = build_branch_inventory_snapshot_entries(
            vec![make_branch_info("feature/ambiguous")],
            Vec::new(),
            vec![
                make_inventory_worktree("/tmp/wt-a", "feature/ambiguous"),
                make_inventory_worktree("/tmp/wt-b", "feature/ambiguous"),
            ],
            &[Remote::new("origin", "https://example.com/repo.git")],
        );

        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.worktree_count, 2);
        assert!(entry.worktree_path.is_none());
        assert_eq!(
            entry.resolution_action,
            BranchInventoryResolutionAction::ResolveAmbiguity
        );
    }

    // --- display_name tests ---

    #[test]
    fn test_branch_info_default_display_name_none() {
        let branch = gwt_core::git::Branch {
            name: "feature/test".to_string(),
            commit: "abc1234".to_string(),
            is_current: false,
            has_remote: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
            is_gone: false,
        };
        let info = BranchInfo::from(branch);
        assert_eq!(info.display_name, None);
    }

    #[test]
    fn test_branch_info_serializes_display_name() {
        let branch = gwt_core::git::Branch {
            name: "feature/test".to_string(),
            commit: "abc1234".to_string(),
            is_current: false,
            has_remote: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
            is_gone: false,
        };
        let mut info = BranchInfo::from(branch);
        info.display_name = Some("My feature".to_string());

        let json = serde_json::to_string(&info).unwrap();
        assert!(
            json.contains(r#""display_name":"My feature""#),
            "JSON should contain display_name with value: {}",
            json
        );
    }

    #[test]
    fn test_branch_info_serializes_null_display_name() {
        let branch = gwt_core::git::Branch {
            name: "feature/test".to_string(),
            commit: "abc1234".to_string(),
            is_current: false,
            has_remote: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            commit_timestamp: None,
            is_gone: false,
        };
        let info = BranchInfo::from(branch);

        let json = serde_json::to_string(&info).unwrap();
        assert!(
            json.contains(r#""display_name":null"#),
            "JSON should contain display_name:null: {}",
            json
        );
    }

    #[test]
    fn test_normalize_generated_display_name_strips_inferred_prefix_and_suffix() {
        assert_eq!(
            normalize_generated_display_name(
                "（推定）認証フローのエラーハンドリングを改善すること",
                "feature/auth-flow"
            ),
            Some("認証フローのエラーハンドリングを改善する".to_string())
        );
    }

    #[test]
    fn test_normalize_generated_display_name_humanizes_branch_fallback_text() {
        assert_eq!(
            normalize_generated_display_name(
                "(Inferred) Deliver the outcome intended by branch 'feature/issue-1644'",
                "feature/issue-1644"
            ),
            Some("#1644".to_string())
        );
    }

    #[test]
    fn test_normalize_generated_display_name_returns_none_for_unknown_text() {
        assert_eq!(
            normalize_generated_display_name("Unknown", "feature/issue-1644"),
            None
        );
    }

    #[test]
    fn test_normalize_generated_display_name_rejects_code_fragment() {
        assert_eq!(
            normalize_generated_display_name(
                "&str, lang: SummaryLanguage) -> Option<String>",
                "feature/issue-1644"
            ),
            None
        );
    }

    #[test]
    fn test_build_issue_display_name_map_with_formats_issue_title() {
        let names = ["feature/issue-1644".to_string(), "develop".to_string()];
        let issue_names = build_issue_display_name_map_with(names.iter(), |number| {
            assert_eq!(number, 1644);
            Ok((1644, "Worktree管理".to_string()))
        });

        assert_eq!(
            issue_names.get("feature/issue-1644"),
            Some(&"#1644 Worktree管理".to_string())
        );
        assert!(!issue_names.contains_key("develop"));
    }
    #[test]
    fn test_build_issue_display_name_map_uses_cached_issue_titles_only() {
        let temp = TempDir::new().unwrap();
        let repo_path = temp.path().join("repo");
        std::fs::create_dir_all(&repo_path).unwrap();

        let state = AppState::new();
        let repo_key = repo_path.to_string_lossy().to_string();
        let response_json = serde_json::json!({
            "issues": [
                {
                    "number": 1644,
                    "title": "Worktree管理",
                    "updatedAt": "2026-03-18T00:00:00Z",
                    "labels": [],
                    "body": null,
                    "state": "open",
                    "htmlUrl": "https://example.com/issues/1644",
                    "assignees": [],
                    "commentsCount": 0,
                    "milestone": null
                }
            ],
            "hasNextPage": false
        })
        .to_string();
        state.project_issue_list_cache.lock().unwrap().insert(
            repo_key,
            HashMap::from([(
                "page=1&per_page=50&state=open&category=all&include_body=false".to_string(),
                IssueListCacheEntry {
                    fetched_at_millis: 0,
                    response_json,
                },
            )]),
        );

        let names = vec![
            "feature/issue-1644".to_string(),
            "feature/issue-2000".to_string(),
        ];
        let issue_names = build_issue_display_name_map(&names, &repo_path, &state);

        assert_eq!(
            issue_names.get("feature/issue-1644"),
            Some(&"#1644 Worktree管理".to_string())
        );
        assert!(!issue_names.contains_key("feature/issue-2000"));
    }

    #[test]
    fn test_resolve_auto_display_name_prefers_issue_title_over_ai_summary() {
        let mut issue_names = HashMap::new();
        issue_names.insert(
            "feature/issue-1644".to_string(),
            "#1644 Worktree管理".to_string(),
        );

        assert_eq!(
            resolve_auto_display_name(
                "feature/issue-1644",
                &issue_names,
                Some("認証フローのエラーハンドリングを改善すること")
            ),
            Some("#1644 Worktree管理".to_string())
        );
    }

    #[test]
    fn test_resolve_auto_display_name_keeps_develop_raw() {
        let mut issue_names = HashMap::new();
        issue_names.insert("develop".to_string(), "#9999 Should not use".to_string());

        assert_eq!(
            resolve_auto_display_name(
                "develop",
                &issue_names,
                Some("認証フローのエラーハンドリングを改善すること")
            ),
            None
        );
    }
}
