//! Session history commands (Quick Start)

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::commands::terminal::capture_scrollback_tail_from_state;
use crate::state::AppState;
use gwt_core::ai::{
    format_error_for_display, summarize_scrollback, summarize_session, AIClient, AIError,
    AgentType as AiAgentType, ClaudeSessionParser, CodexSessionParser, GeminiSessionParser,
    OpenCodeSessionParser, SessionParseError, SessionParser, SessionSummary,
};
use gwt_core::config::{ProfilesConfig, ResolvedAISettings, ToolSessionEntry};
use gwt_core::terminal::pane::PaneStatus;
use gwt_core::terminal::scrollback::ScrollbackFile;
use serde::Serialize;
use std::collections::HashSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager, State};

/// Return tool-specific latest session entries for a branch (Quick Start).
///
/// This is a read-only operation (no config/history writes).
#[tauri::command]
pub fn get_branch_quick_start(
    project_path: String,
    branch: String,
) -> Result<Vec<ToolSessionEntry>, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Branch is required".to_string());
    }

    Ok(gwt_core::config::get_branch_tool_history(
        &repo_path, branch,
    ))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSidebarSubAgent {
    pub id: String,
    pub name: String,
    pub tool_id: String,
    pub status: String, // "running" | "completed" | "failed"
    pub model: Option<String>,
    pub branch: String,
    pub worktree_rel_path: String,
    pub worktree_abs_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSidebarTask {
    pub id: String,
    pub title: String,
    pub status: String, // "running" | "pending" | "failed" | "completed"
    pub sub_agents: Vec<AgentSidebarSubAgent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSidebarView {
    pub spec_id: Option<String>,
    pub tasks: Vec<AgentSidebarTask>,
}

#[derive(Debug, Clone)]
struct ParsedTask {
    id: String,
    title: String,
    base_status: String, // "pending" | "completed"
}

#[derive(Debug, Clone)]
struct ParsedTaskSet {
    spec_id: Option<String>,
    tasks: Vec<ParsedTask>,
}

#[derive(Debug, Clone)]
struct RunningPaneRef {
    branch: String,
    tool_id: String,
}

#[tauri::command]
pub fn get_agent_sidebar_view(
    project_path: String,
    state: State<AppState>,
) -> Result<AgentSidebarView, String> {
    let project_root = Path::new(&project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;

    let parsed = parse_latest_spec_tasks(project_root)?;
    let entries = load_recent_sub_agents(&repo_path);
    let running_refs = collect_running_pane_refs(&state, &repo_path);

    let mut tasks: Vec<AgentSidebarTask> = parsed
        .tasks
        .into_iter()
        .map(|t| AgentSidebarTask {
            id: t.id,
            title: t.title,
            status: t.base_status,
            sub_agents: Vec::new(),
        })
        .collect();

    if tasks.is_empty() && !entries.is_empty() {
        for (idx, entry) in entries.iter().enumerate() {
            tasks.push(AgentSidebarTask {
                id: format!("TASK-{}", idx + 1),
                title: format!(
                    "{} ({})",
                    display_tool_name(entry),
                    normalize_branch_name(&entry.branch)
                ),
                status: "pending".to_string(),
                sub_agents: Vec::new(),
            });
        }
    }
    for (idx, entry) in entries.iter().enumerate() {
        let sub = map_entry_to_sub_agent(entry, idx, project_root, &repo_path, &running_refs);
        let task_idx = detect_task_index_for_entry(entry, &tasks)
            .unwrap_or_else(|| fallback_task_index(&tasks));
        if let Some(task) = tasks.get_mut(task_idx) {
            task.sub_agents.push(sub);
        }
    }

    for task in &mut tasks {
        let has_running = task.sub_agents.iter().any(|a| a.status == "running");
        let has_failed = task.sub_agents.iter().any(|a| a.status == "failed");
        let next_status = if has_running {
            "running"
        } else if task.status == "completed" {
            "completed"
        } else if has_failed {
            "failed"
        } else {
            "pending"
        };
        task.status = next_status.to_string();
    }

    tasks.sort_by(|a, b| {
        task_status_rank(&a.status)
            .cmp(&task_status_rank(&b.status))
            .then_with(|| a.id.cmp(&b.id))
    });

    Ok(AgentSidebarView {
        spec_id: parsed.spec_id,
        tasks,
    })
}

fn parse_latest_spec_tasks(project_root: &Path) -> Result<ParsedTaskSet, String> {
    let specs_root = project_root.join("specs");
    if !specs_root.exists() {
        return Ok(ParsedTaskSet {
            spec_id: None,
            tasks: Vec::new(),
        });
    }

    let mut newest: Option<(SystemTime, String, std::path::PathBuf)> = None;
    let entries = fs::read_dir(&specs_root).map_err(|e| format!("Failed to read specs/: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("Failed to read specs entry: {e}"))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to read specs entry type: {e}"))?;
        if !file_type.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("SPEC-") {
            continue;
        }

        let tasks_path = entry.path().join("tasks.md");
        if !tasks_path.exists() {
            continue;
        }

        let modified = fs::metadata(&tasks_path)
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        match &newest {
            Some((current, _, _)) if &modified <= current => {}
            _ => newest = Some((modified, name, tasks_path)),
        }
    }

    let Some((_, spec_id, tasks_path)) = newest else {
        return Ok(ParsedTaskSet {
            spec_id: None,
            tasks: Vec::new(),
        });
    };

    let content =
        fs::read_to_string(&tasks_path).map_err(|e| format!("Failed to read tasks.md: {e}"))?;
    let tasks = parse_tasks_markdown(&content);

    Ok(ParsedTaskSet {
        spec_id: Some(spec_id),
        tasks,
    })
}

fn parse_tasks_markdown(content: &str) -> Vec<ParsedTask> {
    let mut out = Vec::new();

    for line in content.lines() {
        let trimmed = line.trim_start();
        let (is_completed, body) = if let Some(rest) = trimmed.strip_prefix("- [ ]") {
            (false, rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix("- [x]") {
            (true, rest.trim())
        } else if let Some(rest) = trimmed.strip_prefix("- [X]") {
            (true, rest.trim())
        } else {
            continue;
        };

        if body.is_empty() {
            continue;
        }

        let mut id: Option<String> = None;
        for token in body.split_whitespace() {
            let normalized = token.trim_matches(|c: char| !c.is_ascii_alphanumeric());
            if looks_like_task_id(normalized) {
                id = Some(normalized.to_ascii_uppercase());
                break;
            }
        }
        let id = id.unwrap_or_else(|| format!("TASK-{}", out.len() + 1));

        out.push(ParsedTask {
            id,
            title: body.to_string(),
            base_status: if is_completed {
                "completed".to_string()
            } else {
                "pending".to_string()
            },
        });
    }

    out
}

fn looks_like_task_id(token: &str) -> bool {
    if token.len() < 2 {
        return false;
    }
    if !token.starts_with('T') {
        return false;
    }
    token[1..].chars().all(|c| c.is_ascii_digit())
}

fn load_recent_sub_agents(repo_path: &Path) -> Vec<ToolSessionEntry> {
    let out = gwt_core::config::load_ts_session(repo_path)
        .map(|s| s.history)
        .unwrap_or_default();
    dedupe_current_sub_agents(out)
}

fn dedupe_current_sub_agents(mut entries: Vec<ToolSessionEntry>) -> Vec<ToolSessionEntry> {
    // Keep only the latest assignment per (tool, branch).
    entries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    let mut deduped = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let key = format!(
            "{}::{}",
            normalize_tool_id(&entry.tool_id),
            normalize_branch_name(&entry.branch)
        );
        if seen.insert(key) {
            deduped.push(entry);
        }
    }
    deduped
}

fn collect_running_pane_refs(state: &AppState, repo_path: &Path) -> Vec<RunningPaneRef> {
    let panes = match state.pane_manager.lock() {
        Ok(manager) => manager
            .panes()
            .iter()
            .map(|p| {
                (
                    p.pane_id().to_string(),
                    p.branch_name().to_string(),
                    p.status().clone(),
                )
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };
    let meta = state
        .pane_launch_meta
        .lock()
        .ok()
        .map(|m| m.clone())
        .unwrap_or_default();

    let mut out = Vec::new();
    for (pane_id, pane_branch, pane_status) in panes {
        if !matches!(pane_status, PaneStatus::Running) {
            continue;
        }
        let Some(m) = meta.get(&pane_id) else {
            continue;
        };
        if m.repo_path.as_path() != repo_path {
            continue;
        }
        let branch = if m.branch.trim().is_empty() {
            normalize_branch_name(&pane_branch)
        } else {
            normalize_branch_name(&m.branch)
        };
        out.push(RunningPaneRef {
            branch,
            tool_id: normalize_tool_id(&tool_id_for_agent(&m.agent_id)),
        });
    }
    out
}

fn map_entry_to_sub_agent(
    entry: &ToolSessionEntry,
    idx: usize,
    project_root: &Path,
    repo_path: &Path,
    running_refs: &[RunningPaneRef],
) -> AgentSidebarSubAgent {
    let tool_id = normalize_tool_id(&entry.tool_id);
    let branch = normalize_branch_name(&entry.branch);
    let running = running_refs
        .iter()
        .any(|r| r.branch == branch && r.tool_id == tool_id);
    let mut status = if running { "running" } else { "completed" }.to_string();
    if !running {
        if let Some(mode) = entry.mode.as_deref() {
            let m = mode.to_ascii_lowercase();
            if m == "error" || m == "failed" {
                status = "failed".to_string();
            }
        }
    }

    let (worktree_rel_path, worktree_abs_path) =
        derive_worktree_paths(entry.worktree_path.as_deref(), project_root, repo_path);

    AgentSidebarSubAgent {
        id: entry
            .session_id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| format!("sub-agent-{}", idx + 1)),
        name: display_tool_name(entry),
        tool_id,
        status,
        model: entry.model.clone().filter(|m| !m.trim().is_empty()),
        branch,
        worktree_rel_path,
        worktree_abs_path,
    }
}

fn detect_task_index_for_entry(
    entry: &ToolSessionEntry,
    tasks: &[AgentSidebarTask],
) -> Option<usize> {
    let mut haystack = String::new();
    haystack.push_str(&entry.branch.to_ascii_lowercase());
    haystack.push(' ');
    haystack.push_str(
        &entry
            .worktree_path
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase(),
    );
    haystack.push(' ');
    haystack.push_str(
        &entry
            .session_id
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase(),
    );
    haystack.push(' ');
    haystack.push_str(&entry.model.as_deref().unwrap_or("").to_ascii_lowercase());

    for (idx, task) in tasks.iter().enumerate() {
        if haystack.contains(&task.id.to_ascii_lowercase()) {
            return Some(idx);
        }
    }
    None
}

fn fallback_task_index(tasks: &[AgentSidebarTask]) -> usize {
    tasks
        .iter()
        .position(|t| t.status != "completed")
        .unwrap_or(0)
}

fn derive_worktree_paths(
    maybe_abs: Option<&str>,
    project_root: &Path,
    repo_path: &Path,
) -> (String, Option<String>) {
    let Some(abs) = maybe_abs.map(str::trim).filter(|s| !s.is_empty()) else {
        return ("-".to_string(), None);
    };
    let abs_path = Path::new(abs);

    if let Ok(rel) = abs_path.strip_prefix(project_root) {
        let rel_text = rel.to_string_lossy().to_string();
        return (
            if rel_text.is_empty() {
                ".".to_string()
            } else {
                rel_text
            },
            Some(abs.to_string()),
        );
    }

    if let Ok(rel) = abs_path.strip_prefix(repo_path) {
        let rel_text = rel.to_string_lossy().to_string();
        return (
            if rel_text.is_empty() {
                ".".to_string()
            } else {
                rel_text
            },
            Some(abs.to_string()),
        );
    }

    (abs.to_string(), Some(abs.to_string()))
}

fn display_tool_name(entry: &ToolSessionEntry) -> String {
    let id = normalize_tool_id(&entry.tool_id);
    if id == "claude-code" {
        return "Claude".to_string();
    }
    if id == "codex-cli" {
        return "Codex".to_string();
    }
    if id == "gemini-cli" {
        return "Gemini".to_string();
    }
    if id == "opencode" {
        return "OpenCode".to_string();
    }
    entry.tool_label.clone()
}

fn normalize_tool_id(tool_id: &str) -> String {
    let id = tool_id.trim().to_ascii_lowercase();
    match id.as_str() {
        "claude" | "claude-code" => "claude-code".to_string(),
        "codex" | "codex-cli" => "codex-cli".to_string(),
        "gemini" | "gemini-cli" => "gemini-cli".to_string(),
        "opencode" | "open-code" => "opencode".to_string(),
        _ => id,
    }
}

fn normalize_branch_name(branch: &str) -> String {
    let trimmed = branch.trim();
    if let Some(rest) = trimmed.strip_prefix("origin/") {
        return rest.to_string();
    }
    trimmed.to_string()
}

fn task_status_rank(status: &str) -> u8 {
    match status {
        "running" => 0,
        "pending" => 1,
        "failed" => 2,
        "completed" => 3,
        _ => 4,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummaryResult {
    pub status: String, // "ok" | "ai-not-configured" | "disabled" | "no-session" | "error"
    pub generating: bool,
    pub tool_id: Option<String>,
    pub session_id: Option<String>,
    pub markdown: Option<String>,
    pub task_overview: Option<String>,
    pub short_summary: Option<String>,
    pub bullet_points: Vec<String>,
    pub warning: Option<String>,
    pub error: Option<String>,
}

fn ok_summary(tool_id: &str, session_id: &str, summary: &SessionSummary) -> SessionSummaryResult {
    SessionSummaryResult {
        status: "ok".to_string(),
        generating: false,
        tool_id: Some(tool_id.to_string()),
        session_id: Some(session_id.to_string()),
        markdown: summary.markdown.clone(),
        task_overview: summary.task_overview.clone(),
        short_summary: summary.short_summary.clone(),
        bullet_points: summary.bullet_points.clone(),
        warning: None,
        error: None,
    }
}

fn summary_status(
    status: &str,
    tool_id: Option<String>,
    session_id: Option<String>,
    message: Option<String>,
) -> SessionSummaryResult {
    SessionSummaryResult {
        status: status.to_string(),
        generating: false,
        tool_id,
        session_id,
        markdown: None,
        task_overview: None,
        short_summary: None,
        bullet_points: Vec::new(),
        warning: None,
        error: message,
    }
}

fn generating_summary(
    tool_id: &str,
    session_id: &str,
    previous: Option<&SessionSummary>,
) -> SessionSummaryResult {
    if let Some(prev) = previous {
        let mut out = ok_summary(tool_id, session_id, prev);
        out.generating = true;
        return out;
    }

    SessionSummaryResult {
        status: "ok".to_string(),
        generating: true,
        tool_id: Some(tool_id.to_string()),
        session_id: Some(session_id.to_string()),
        markdown: None,
        task_overview: None,
        short_summary: None,
        bullet_points: Vec::new(),
        warning: None,
        error: None,
    }
}

fn session_parser_for_tool(tool_id: &str) -> Option<Box<dyn SessionParser>> {
    let agent_type = AiAgentType::from_tool_id(tool_id)?;
    match agent_type {
        AiAgentType::ClaudeCode => {
            ClaudeSessionParser::with_default_home().map(|p| Box::new(p) as Box<dyn SessionParser>)
        }
        AiAgentType::CodexCli => {
            CodexSessionParser::with_default_home().map(|p| Box::new(p) as Box<dyn SessionParser>)
        }
        AiAgentType::GeminiCli => {
            GeminiSessionParser::with_default_home().map(|p| Box::new(p) as Box<dyn SessionParser>)
        }
        AiAgentType::OpenCode => OpenCodeSessionParser::with_default_home()
            .map(|p| Box::new(p) as Box<dyn SessionParser>),
    }
}

fn tool_id_for_agent(agent_id: &str) -> String {
    match agent_id {
        "claude" => "claude-code".to_string(),
        "codex" => "codex-cli".to_string(),
        "gemini" => "gemini-cli".to_string(),
        "opencode" => "opencode".to_string(),
        _ => agent_id.to_string(),
    }
}

fn pane_session_id(pane_id: &str) -> String {
    format!("pane:{pane_id}")
}

fn system_time_millis(time: SystemTime) -> u128 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
struct SessionSummaryJob {
    project_path: String,
    repo_key: String,
    branch: String,
    tool_id: String,
    session_id: String,
    settings: ResolvedAISettings,
    mtime: SystemTime,
}

#[derive(Debug, Clone)]
struct ScrollbackSummaryJob {
    project_path: String,
    repo_key: String,
    branch: String,
    pane_id: String,
    tool_id: String,
    settings: ResolvedAISettings,
    mtime: SystemTime,
}

#[derive(Debug, Clone)]
enum SummaryJob {
    Session(SessionSummaryJob),
    Scrollback(ScrollbackSummaryJob),
}

#[derive(Debug, Clone)]
struct ScrollbackCandidate {
    pane_id: String,
    tool_id: String,
    mtime: SystemTime,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummaryUpdatedPayload {
    pub project_path: String,
    pub branch: String,
    pub result: SessionSummaryResult,
}

fn scrollback_mtime_for_pane(pane_id: &str) -> Option<SystemTime> {
    let path = ScrollbackFile::scrollback_path_for_pane(pane_id).ok()?;
    let metadata = std::fs::metadata(&path).ok()?;
    metadata.modified().ok()
}

fn collect_scrollback_candidates(
    state: &AppState,
    repo_path: &Path,
    branch: &str,
    fallback_tool_id: Option<&str>,
) -> Vec<ScrollbackCandidate> {
    let panes = match state.pane_manager.lock() {
        Ok(manager) => manager
            .panes()
            .iter()
            .map(|pane| {
                (
                    pane.pane_id().to_string(),
                    pane.branch_name().to_string(),
                    pane.status().clone(),
                )
            })
            .collect::<Vec<_>>(),
        Err(_) => Vec::new(),
    };

    let launch_meta = state
        .pane_launch_meta
        .lock()
        .ok()
        .map(|m| m.clone())
        .unwrap_or_default();

    let mut out = Vec::new();
    for (pane_id, pane_branch, status) in panes {
        if pane_branch != branch {
            continue;
        }
        if !matches!(status, PaneStatus::Running) {
            continue;
        }

        let mut tool_id = fallback_tool_id.unwrap_or("").to_string();
        let mut include = true;
        if let Some(meta) = launch_meta.get(&pane_id) {
            if meta.repo_path.as_path() != repo_path {
                include = false;
            } else {
                tool_id = tool_id_for_agent(&meta.agent_id);
            }
        }
        if !include {
            continue;
        }
        if tool_id.trim().is_empty() {
            tool_id = "unknown".to_string();
        }

        let Some(mtime) = scrollback_mtime_for_pane(&pane_id) else {
            continue;
        };

        out.push(ScrollbackCandidate {
            pane_id,
            tool_id,
            mtime,
        });
    }

    out
}

fn select_latest_scrollback_candidate(
    candidates: Vec<ScrollbackCandidate>,
) -> Option<ScrollbackCandidate> {
    candidates
        .into_iter()
        .max_by_key(|c| system_time_millis(c.mtime))
}

fn latest_scrollback_candidate_for_branch(
    state: &AppState,
    repo_path: &Path,
    branch: &str,
    fallback_tool_id: Option<&str>,
) -> Option<ScrollbackCandidate> {
    let candidates = collect_scrollback_candidates(state, repo_path, branch, fallback_tool_id);
    select_latest_scrollback_candidate(candidates)
}

fn is_latest_scrollback_candidate(
    state: &AppState,
    repo_path: &Path,
    branch: &str,
    pane_id: &str,
) -> bool {
    let Some(candidate) = latest_scrollback_candidate_for_branch(state, repo_path, branch, None)
    else {
        return false;
    };
    candidate.pane_id == pane_id
}

fn scrollback_summary_immediate(
    project_path: &str,
    repo_key: &str,
    branch: &str,
    candidate: ScrollbackCandidate,
    settings: ResolvedAISettings,
    state: &AppState,
) -> (SessionSummaryResult, Option<ScrollbackSummaryJob>) {
    let pane_session = pane_session_id(&candidate.pane_id);

    let (cached_ok, previous_any) = {
        let cache_guard = match state.session_summary_cache.lock() {
            Ok(guard) => guard,
            Err(_) => {
                return (
                    summary_status(
                        "error",
                        Some(candidate.tool_id),
                        Some(pane_session),
                        Some("Session summary cache lock poisoned".to_string()),
                    ),
                    None,
                )
            }
        };
        let cache = cache_guard.get(repo_key);
        let cached_ok = cache.and_then(|c| {
            c.get(branch)
                .cloned()
                .filter(|_| !c.is_stale(branch, &pane_session, candidate.mtime))
        });
        let previous_any = cache.and_then(|c| c.get(branch).cloned());
        (cached_ok, previous_any)
    };

    if let Some(summary) = cached_ok.as_ref() {
        return (ok_summary(&candidate.tool_id, &pane_session, summary), None);
    }

    let immediate = generating_summary(&candidate.tool_id, &pane_session, previous_any.as_ref());
    let job = ScrollbackSummaryJob {
        project_path: project_path.to_string(),
        repo_key: repo_key.to_string(),
        branch: branch.to_string(),
        pane_id: candidate.pane_id,
        tool_id: candidate.tool_id,
        settings,
        mtime: candidate.mtime,
    };

    (immediate, Some(job))
}

fn get_branch_session_summary_immediate(
    project_path: &str,
    branch: &str,
    state: &AppState,
) -> Result<(SessionSummaryResult, Option<SummaryJob>), String> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let repo_key = repo_path.to_string_lossy().to_string();

    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Branch is required".to_string());
    }

    let entries = gwt_core::config::get_branch_tool_history(&repo_path, branch);
    let entry = entries.first();

    let tool_id = entry.map(|e| e.tool_id.trim()).unwrap_or("").to_string();
    let session_id = entry
        .and_then(|e| e.session_id.as_deref())
        .unwrap_or("")
        .trim()
        .to_string();

    if tool_id.is_empty() || session_id.is_empty() {
        let candidate = latest_scrollback_candidate_for_branch(
            state,
            &repo_path,
            branch,
            if tool_id.is_empty() {
                None
            } else {
                Some(tool_id.as_str())
            },
        );
        let Some(candidate) = candidate else {
            return Ok((
                summary_status(
                    "no-session",
                    if tool_id.is_empty() {
                        None
                    } else {
                        Some(tool_id)
                    },
                    None,
                    None,
                ),
                None,
            ));
        };

        let profiles = ProfilesConfig::load().map_err(|e| e.to_string())?;
        let ai = profiles.resolve_active_ai_settings();

        if !ai.ai_enabled {
            return Ok((
                summary_status("ai-not-configured", Some(candidate.tool_id), None, None),
                None,
            ));
        }
        if !ai.summary_enabled {
            return Ok((
                summary_status("disabled", Some(candidate.tool_id), None, None),
                None,
            ));
        }

        let settings = ai
            .resolved
            .ok_or_else(|| "AI settings are not configured".to_string())?;

        let (immediate, job) = scrollback_summary_immediate(
            project_path,
            &repo_key,
            branch,
            candidate,
            settings,
            state,
        );
        let job = job.map(SummaryJob::Scrollback);
        return Ok((immediate, job));
    }

    let profiles = ProfilesConfig::load().map_err(|e| e.to_string())?;
    let ai = profiles.resolve_active_ai_settings();

    if !ai.ai_enabled {
        return Ok((
            summary_status("ai-not-configured", Some(tool_id), Some(session_id), None),
            None,
        ));
    }
    if !ai.summary_enabled {
        return Ok((
            summary_status("disabled", Some(tool_id), Some(session_id), None),
            None,
        ));
    }

    let settings = ai
        .resolved
        .ok_or_else(|| "AI settings are not configured".to_string())?;
    let parser = match session_parser_for_tool(&tool_id) {
        Some(p) => p,
        None => {
            return Ok((
                summary_status(
                    "error",
                    Some(tool_id),
                    Some(session_id),
                    Some("Unsupported agent session".to_string()),
                ),
                None,
            ))
        }
    };

    let path = parser.session_file_path(&session_id);
    let metadata = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(err) => {
            // If we already have a cached summary, keep showing it even if the session file
            // is temporarily missing (best-effort UX).
            let previous_any = {
                let cache_guard = state
                    .session_summary_cache
                    .lock()
                    .map_err(|_| "Session summary cache lock poisoned".to_string())?;
                cache_guard
                    .get(&repo_key)
                    .and_then(|c| c.get(branch).cloned())
            };

            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&tool_id, &session_id, prev);
                out.warning = Some(format!(
                    "Failed to read session file; keeping previous: {err}"
                ));
                return Ok((out, None));
            }

            let missing = err.kind() == std::io::ErrorKind::NotFound;
            return Ok((
                summary_status(
                    if missing { "no-session" } else { "error" },
                    Some(tool_id),
                    Some(session_id),
                    Some(err.to_string()),
                ),
                None,
            ));
        }
    };
    let mtime = metadata.modified().unwrap_or_else(|_| SystemTime::now());

    // Cache lookup (best-effort). Do not hold the mutex while doing network calls.
    let (cached_ok, previous_any) = {
        let cache_guard = state
            .session_summary_cache
            .lock()
            .map_err(|_| "Session summary cache lock poisoned".to_string())?;
        let cache = cache_guard.get(&repo_key);
        let cached_ok = cache.and_then(|c| {
            c.get(branch)
                .cloned()
                .filter(|_| !c.is_stale(branch, &session_id, mtime))
        });
        let previous_any = cache.and_then(|c| c.get(branch).cloned());
        (cached_ok, previous_any)
    };

    if let Some(summary) = cached_ok.as_ref() {
        return Ok((ok_summary(&tool_id, &session_id, summary), None));
    }

    let immediate = generating_summary(&tool_id, &session_id, previous_any.as_ref());
    let job = SessionSummaryJob {
        project_path: project_path.to_string(),
        repo_key,
        branch: branch.to_string(),
        tool_id,
        session_id,
        settings,
        mtime,
    };

    Ok((immediate, Some(SummaryJob::Session(job))))
}

pub(crate) fn prewarm_missing_worktree_summaries(
    project_path: String,
    branches: Vec<String>,
    app_handle: AppHandle,
) {
    if branches.is_empty() {
        return;
    }

    let state = app_handle.state::<AppState>();
    let mut seen = HashSet::new();

    for branch in branches {
        let branch = branch.trim().to_string();
        if branch.is_empty() || !seen.insert(branch.clone()) {
            continue;
        }

        let Ok((_, maybe_job)) =
            get_branch_session_summary_immediate(&project_path, &branch, &state)
        else {
            continue;
        };

        match maybe_job {
            Some(SummaryJob::Session(job)) => {
                start_session_summary_job(job, &state, app_handle.clone())
            }
            Some(SummaryJob::Scrollback(job)) => {
                start_scrollback_summary_job(job, &state, app_handle.clone())
            }
            None => {}
        }
    }
}

fn is_latest_branch_session(repo_key: &str, branch: &str, tool_id: &str, session_id: &str) -> bool {
    let entries = gwt_core::config::get_branch_tool_history(Path::new(repo_key), branch);
    let Some(entry) = entries.first() else {
        // If we can't determine the current session, treat as latest to avoid breaking updates.
        return true;
    };

    let current_tool_id = entry.tool_id.trim();
    let current_session_id = entry.session_id.as_deref().unwrap_or("").trim();
    if current_tool_id.is_empty() || current_session_id.is_empty() {
        return true;
    }

    current_tool_id == tool_id && current_session_id == session_id
}

fn start_session_summary_job(job: SessionSummaryJob, state: &AppState, app_handle: AppHandle) {
    let inflight_key = format!(
        "{}::{}::{}::{}",
        job.repo_key, job.branch, job.tool_id, job.session_id
    );
    let should_spawn = match state.session_summary_inflight.lock() {
        Ok(mut set) => {
            if set.contains(&inflight_key) {
                false
            } else {
                set.insert(inflight_key.clone());
                true
            }
        }
        Err(_) => false,
    };

    if !should_spawn {
        return;
    }

    let app_handle_clone = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle_clone.state::<AppState>();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            generate_and_cache_session_summary(&job, &state)
        }))
        .unwrap_or_else(|_| {
            summary_status(
                "error",
                Some(job.tool_id.clone()),
                Some(job.session_id.clone()),
                Some("Internal error".to_string()),
            )
        });

        if let Ok(mut set) = state.session_summary_inflight.lock() {
            set.remove(&inflight_key);
        }

        // If the branch has moved to a different latest session while this job was running,
        // skip emitting an update event to avoid clobbering the UI with stale data.
        if !is_latest_branch_session(&job.repo_key, &job.branch, &job.tool_id, &job.session_id) {
            return;
        }

        let payload = SessionSummaryUpdatedPayload {
            project_path: job.project_path.clone(),
            branch: job.branch.clone(),
            result,
        };
        let _ = app_handle_clone.emit("session-summary-updated", &payload);
    });
}

fn start_scrollback_summary_job(
    job: ScrollbackSummaryJob,
    state: &AppState,
    app_handle: AppHandle,
) {
    let inflight_key = format!(
        "scrollback::{}::{}::{}",
        job.repo_key, job.branch, job.pane_id
    );
    let should_spawn = match state.session_summary_inflight.lock() {
        Ok(mut set) => {
            if set.contains(&inflight_key) {
                false
            } else {
                set.insert(inflight_key.clone());
                true
            }
        }
        Err(_) => false,
    };

    if !should_spawn {
        return;
    }

    let app_handle_clone = app_handle.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app_handle_clone.state::<AppState>();

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            generate_and_cache_scrollback_summary(&job, &state)
        }))
        .unwrap_or_else(|_| {
            summary_status(
                "error",
                Some(job.tool_id.clone()),
                Some(pane_session_id(&job.pane_id)),
                Some("Internal error".to_string()),
            )
        });

        if let Ok(mut set) = state.session_summary_inflight.lock() {
            set.remove(&inflight_key);
        }

        if !is_latest_scrollback_candidate(
            &state,
            Path::new(&job.repo_key),
            &job.branch,
            &job.pane_id,
        ) {
            return;
        }

        let payload = SessionSummaryUpdatedPayload {
            project_path: job.project_path.clone(),
            branch: job.branch.clone(),
            result,
        };
        let _ = app_handle_clone.emit("session-summary-updated", &payload);
    });
}

fn generate_and_cache_session_summary(
    job: &SessionSummaryJob,
    state: &AppState,
) -> SessionSummaryResult {
    // Cache lookup for fallback (best-effort).
    let previous_any = state.session_summary_cache.lock().ok().and_then(|guard| {
        guard
            .get(&job.repo_key)
            .and_then(|c| c.get(&job.branch).cloned())
    });

    let parser = match session_parser_for_tool(&job.tool_id) {
        Some(p) => p,
        None => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &job.session_id, prev);
                out.warning = Some("Unsupported agent session; keeping previous".to_string());
                return out;
            }
            return summary_status(
                "error",
                Some(job.tool_id.clone()),
                Some(job.session_id.clone()),
                Some("Unsupported agent session".to_string()),
            );
        }
    };

    let parsed = match parser.parse(&job.session_id) {
        Ok(parsed) => parsed,
        Err(err) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &job.session_id, prev);
                out.warning = Some(format!("Failed to parse session; keeping previous: {err}"));
                return out;
            }
            let missing = matches!(err, SessionParseError::FileNotFound(_));
            return summary_status(
                if missing { "no-session" } else { "error" },
                Some(job.tool_id.clone()),
                Some(job.session_id.clone()),
                Some(err.to_string()),
            );
        }
    };

    let client = match AIClient::new(job.settings.clone()) {
        Ok(client) => client,
        Err(err) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &job.session_id, prev);
                out.warning = Some(format!("Failed to initialize AI; keeping previous: {err}"));
                return out;
            }
            return summary_status(
                "error",
                Some(job.tool_id.clone()),
                Some(job.session_id.clone()),
                Some(err.to_string()),
            );
        }
    };

    match summarize_session(&client, &parsed) {
        Ok(summary) => {
            // Avoid overwriting the cache if the branch's latest session has changed
            // since the job started (e.g., a new session was recorded).
            if is_latest_branch_session(&job.repo_key, &job.branch, &job.tool_id, &job.session_id) {
                if let Ok(mut cache_guard) = state.session_summary_cache.lock() {
                    cache_guard.entry(job.repo_key.clone()).or_default().set(
                        job.branch.clone(),
                        job.session_id.clone(),
                        summary.clone(),
                        job.mtime,
                    );
                }
            }
            ok_summary(&job.tool_id, &job.session_id, &summary)
        }
        Err(AIError::IncompleteSummary) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &job.session_id, prev);
                out.warning = Some("Incomplete summary; keeping previous".to_string());
                out
            } else {
                summary_status(
                    "error",
                    Some(job.tool_id.clone()),
                    Some(job.session_id.clone()),
                    Some(format_error_for_display(&AIError::IncompleteSummary)),
                )
            }
        }
        Err(other) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &job.session_id, prev);
                out.warning = Some(format!(
                    "Update failed; keeping previous: {}",
                    format_error_for_display(&other)
                ));
                out
            } else {
                summary_status(
                    "error",
                    Some(job.tool_id.clone()),
                    Some(job.session_id.clone()),
                    Some(format_error_for_display(&other)),
                )
            }
        }
    }
}

fn generate_and_cache_scrollback_summary(
    job: &ScrollbackSummaryJob,
    state: &AppState,
) -> SessionSummaryResult {
    let previous_any = state.session_summary_cache.lock().ok().and_then(|guard| {
        guard
            .get(&job.repo_key)
            .and_then(|c| c.get(&job.branch).cloned())
    });

    let pane_session = pane_session_id(&job.pane_id);
    let scrollback = match capture_scrollback_tail_from_state(state, &job.pane_id, 0) {
        Ok(text) => text,
        Err(err) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &pane_session, prev);
                out.warning = Some(format!(
                    "Failed to read scrollback; keeping previous: {err}"
                ));
                return out;
            }
            return summary_status(
                "error",
                Some(job.tool_id.clone()),
                Some(pane_session),
                Some(err),
            );
        }
    };

    let client = match AIClient::new(job.settings.clone()) {
        Ok(client) => client,
        Err(err) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &pane_session, prev);
                out.warning = Some(format!("Failed to initialize AI; keeping previous: {err}"));
                return out;
            }
            return summary_status(
                "error",
                Some(job.tool_id.clone()),
                Some(pane_session),
                Some(err.to_string()),
            );
        }
    };

    match summarize_scrollback(&client, &scrollback, &job.branch) {
        Ok(summary) => {
            if is_latest_scrollback_candidate(
                state,
                Path::new(&job.repo_key),
                &job.branch,
                &job.pane_id,
            ) {
                if let Ok(mut cache_guard) = state.session_summary_cache.lock() {
                    cache_guard.entry(job.repo_key.clone()).or_default().set(
                        job.branch.clone(),
                        pane_session.clone(),
                        summary.clone(),
                        job.mtime,
                    );
                }
            }
            ok_summary(&job.tool_id, &pane_session, &summary)
        }
        Err(AIError::IncompleteSummary) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &pane_session, prev);
                out.warning = Some("Incomplete summary; keeping previous".to_string());
                out
            } else {
                summary_status(
                    "error",
                    Some(job.tool_id.clone()),
                    Some(pane_session),
                    Some(format_error_for_display(&AIError::IncompleteSummary)),
                )
            }
        }
        Err(other) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&job.tool_id, &pane_session, prev);
                out.warning = Some(format!(
                    "Update failed; keeping previous: {}",
                    format_error_for_display(&other)
                ));
                out
            } else {
                summary_status(
                    "error",
                    Some(job.tool_id.clone()),
                    Some(pane_session),
                    Some(format_error_for_display(&other)),
                )
            }
        }
    }
}

/// Return (and cache) an AI session summary for the selected branch.
///
/// - Uses the latest ToolSessionEntry for the branch (most recent tool usage).
/// - Reads agent session file via the tool-specific session parser.
/// - Summarizes using the active AI profile settings when enabled.
/// - Never writes settings/history files as a side effect.
#[tauri::command]
pub fn get_branch_session_summary(
    project_path: String,
    branch: String,
    state: State<AppState>,
    app_handle: AppHandle,
) -> Result<SessionSummaryResult, String> {
    let (result, job) = get_branch_session_summary_immediate(&project_path, &branch, &state)?;
    if let Some(job) = job {
        match job {
            SummaryJob::Session(job) => start_session_summary_job(job, &state, app_handle),
            SummaryJob::Scrollback(job) => start_scrollback_summary_job(job, &state, app_handle),
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{TestEnvGuard, ENV_LOCK};
    use std::fs;
    use std::path::Path;
    use std::time::Duration;
    use tempfile::TempDir;

    fn init_git_repo(path: &Path) {
        let out = gwt_core::process::command("git")
            .args(["init"])
            .current_dir(path)
            .output();
        assert!(out.is_ok(), "git init failed to run");
        assert!(
            out.unwrap().status.success(),
            "git init failed to create repo"
        );
    }

    fn write_session_entry(repo_root: &Path, branch: &str, tool_id: &str, session_id: &str) {
        let entry = ToolSessionEntry {
            branch: branch.to_string(),
            worktree_path: None,
            tool_id: tool_id.to_string(),
            tool_label: "Codex".to_string(),
            session_id: Some(session_id.to_string()),
            mode: None,
            model: None,
            reasoning_level: None,
            skip_permissions: None,
            tool_version: Some("latest".to_string()),
            collaboration_modes: None,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            docker_container_name: None,
            docker_compose_args: None,
            timestamp: 1,
        };

        gwt_core::config::save_session_entry(repo_root, entry).expect("save session entry");
    }

    #[test]
    fn parse_tasks_markdown_extracts_status_and_ids() {
        let tasks = parse_tasks_markdown(
            r#"
# タスク
- [ ] T001 [US1] implement api
- [x] T002 [US2] add tests
- [ ] no-id fallback task
"#,
        );
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].id, "T001");
        assert_eq!(tasks[0].base_status, "pending");
        assert_eq!(tasks[1].id, "T002");
        assert_eq!(tasks[1].base_status, "completed");
        assert_eq!(tasks[2].id, "TASK-3");
    }

    #[test]
    fn derive_worktree_paths_prefers_project_relative() {
        let project_root = Path::new("/repo");
        let repo_path = Path::new("/repo/bare.git");
        let (rel, abs) =
            derive_worktree_paths(Some("/repo/.worktrees/agent-auth"), project_root, repo_path);
        assert_eq!(rel, ".worktrees/agent-auth");
        assert_eq!(abs.as_deref(), Some("/repo/.worktrees/agent-auth"));
    }

    #[test]
    fn task_status_rank_matches_spec_order() {
        assert!(task_status_rank("running") < task_status_rank("pending"));
        assert!(task_status_rank("pending") < task_status_rank("failed"));
        assert!(task_status_rank("failed") < task_status_rank("completed"));
    }

    #[test]
    fn dedupe_current_sub_agents_keeps_latest_per_tool_and_branch() {
        let mk = |tool_id: &str, branch: &str, session_id: &str, timestamp: i64| ToolSessionEntry {
            branch: branch.to_string(),
            worktree_path: None,
            tool_id: tool_id.to_string(),
            tool_label: "Agent".to_string(),
            session_id: Some(session_id.to_string()),
            mode: None,
            model: None,
            reasoning_level: None,
            skip_permissions: None,
            tool_version: None,
            collaboration_modes: None,
            docker_service: None,
            docker_force_host: None,
            docker_recreate: None,
            docker_build: None,
            docker_keep: None,
            docker_container_name: None,
            docker_compose_args: None,
            timestamp,
        };

        let out = dedupe_current_sub_agents(vec![
            mk("codex-cli", "feature/a", "s-old", 10),
            mk("codex-cli", "feature/a", "s-new", 20),
            mk("claude-code", "feature/a", "s-claude", 15),
            mk("codex-cli", "feature/b", "s-b", 18),
        ]);

        assert_eq!(out.len(), 3);
        let ids: Vec<String> = out
            .iter()
            .map(|e| e.session_id.clone().unwrap_or_default())
            .collect();
        assert!(ids.contains(&"s-new".to_string()));
        assert!(ids.contains(&"s-claude".to_string()));
        assert!(ids.contains(&"s-b".to_string()));
        assert!(!ids.contains(&"s-old".to_string()));
    }

    #[test]
    fn session_summary_returns_no_session_when_history_missing() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());

        let state = AppState::new();
        let (out, job) =
            get_branch_session_summary_immediate(repo.path().to_str().unwrap(), "main", &state)
                .unwrap();
        assert_eq!(out.status, "no-session");
        assert!(!out.generating);
        assert!(job.is_none());
    }

    #[test]
    fn session_summary_returns_ai_not_configured_when_profiles_missing() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());
        write_session_entry(repo.path(), "main", "codex-cli", "session-1");

        let state = AppState::new();
        let (out, job) =
            get_branch_session_summary_immediate(repo.path().to_str().unwrap(), "main", &state)
                .unwrap();
        assert_eq!(out.status, "ai-not-configured");
        assert!(!out.generating);
        assert!(job.is_none());
        assert_eq!(out.tool_id.as_deref(), Some("codex-cli"));
        assert_eq!(out.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn session_summary_returns_disabled_when_summary_disabled() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let mut config = ProfilesConfig::default();
        config.default_ai = Some(gwt_core::config::AISettings {
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-5.2-codex".to_string(),
            summary_enabled: false,
        });
        config.save().unwrap();

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());
        write_session_entry(repo.path(), "main", "codex-cli", "session-1");

        let state = AppState::new();
        let (out, job) =
            get_branch_session_summary_immediate(repo.path().to_str().unwrap(), "main", &state)
                .unwrap();
        assert_eq!(out.status, "disabled");
        assert!(!out.generating);
        assert!(job.is_none());
        assert_eq!(out.tool_id.as_deref(), Some("codex-cli"));
        assert_eq!(out.session_id.as_deref(), Some("session-1"));
    }

    #[test]
    fn session_summary_returns_generating_when_cache_miss_and_session_file_present() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let mut config = ProfilesConfig::default();
        config.default_ai = Some(gwt_core::config::AISettings {
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
            summary_enabled: true,
        });
        config.save().unwrap();

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());
        write_session_entry(repo.path(), "main", "codex-cli", "sess-123");

        let sessions_dir = home.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_path = sessions_dir.join("sess-123.jsonl");
        fs::write(
            &session_path,
            r#"{"payload":{"id":"sess-123","cwd":"/repo/wt"}}"#,
        )
        .unwrap();

        let state = AppState::new();
        let (out, job) =
            get_branch_session_summary_immediate(repo.path().to_str().unwrap(), "main", &state)
                .unwrap();

        assert_eq!(out.status, "ok");
        assert!(out.generating);
        assert_eq!(out.tool_id.as_deref(), Some("codex-cli"));
        assert_eq!(out.session_id.as_deref(), Some("sess-123"));
        assert!(matches!(job, Some(SummaryJob::Session(_))));
    }

    #[test]
    fn session_summary_returns_cached_immediately_when_fresh() {
        let _lock = ENV_LOCK.lock().unwrap();
        let home = TempDir::new().unwrap();
        let _env = TestEnvGuard::new(home.path());

        let mut config = ProfilesConfig::default();
        config.default_ai = Some(gwt_core::config::AISettings {
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
            summary_enabled: true,
        });
        config.save().unwrap();

        let repo = TempDir::new().unwrap();
        init_git_repo(repo.path());
        write_session_entry(repo.path(), "main", "codex-cli", "sess-999");

        let sessions_dir = home.path().join(".codex").join("sessions");
        fs::create_dir_all(&sessions_dir).unwrap();
        let session_path = sessions_dir.join("sess-999.jsonl");
        fs::write(
            &session_path,
            r#"{"payload":{"id":"sess-999","cwd":"/repo/wt"}}"#,
        )
        .unwrap();
        let mtime = fs::metadata(&session_path)
            .unwrap()
            .modified()
            .unwrap_or_else(|_| SystemTime::now());

        let summary = SessionSummary {
            markdown: Some(
                "## Purpose\nCached\n\n## Summary\nCached\n\n## Highlights\n- A\n".to_string(),
            ),
            ..Default::default()
        };

        let state = AppState::new();
        {
            let repo_key = repo.path().to_string_lossy().to_string();
            let mut guard = state.session_summary_cache.lock().unwrap();
            guard.entry(repo_key).or_default().set(
                "main".to_string(),
                "sess-999".to_string(),
                summary,
                mtime,
            );
        }

        let (out, job) =
            get_branch_session_summary_immediate(repo.path().to_str().unwrap(), "main", &state)
                .unwrap();
        assert_eq!(out.status, "ok");
        assert!(!out.generating);
        assert!(job.is_none());
        assert!(out.markdown.as_deref().unwrap_or("").contains("Cached"));
    }

    #[test]
    fn select_latest_scrollback_candidate_picks_latest_mtime() {
        let base = UNIX_EPOCH + Duration::from_secs(10);
        let older = UNIX_EPOCH + Duration::from_secs(1);

        let candidates = vec![
            ScrollbackCandidate {
                pane_id: "pane-1".to_string(),
                tool_id: "codex-cli".to_string(),
                mtime: older,
            },
            ScrollbackCandidate {
                pane_id: "pane-2".to_string(),
                tool_id: "codex-cli".to_string(),
                mtime: base,
            },
        ];

        let selected = select_latest_scrollback_candidate(candidates).unwrap();
        assert_eq!(selected.pane_id, "pane-2");
    }

    #[test]
    fn scrollback_immediate_returns_job_with_pane_session_id() {
        let state = AppState::new();
        let candidate = ScrollbackCandidate {
            pane_id: "pane-xyz".to_string(),
            tool_id: "codex-cli".to_string(),
            mtime: UNIX_EPOCH + Duration::from_secs(5),
        };
        let settings = ResolvedAISettings {
            endpoint: "https://api.openai.com/v1".to_string(),
            api_key: "".to_string(),
            model: "gpt-4o-mini".to_string(),
        };

        let (out, job) = scrollback_summary_immediate(
            "/tmp/project",
            "/tmp/repo",
            "main",
            candidate,
            settings,
            &state,
        );

        assert_eq!(out.status, "ok");
        assert!(out.generating);
        assert!(out.session_id.as_deref().unwrap_or("").starts_with("pane:"));
        assert!(job.is_some());
    }
}
