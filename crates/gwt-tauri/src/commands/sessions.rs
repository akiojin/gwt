//! Session history commands (Quick Start)

use crate::commands::project::resolve_repo_path_for_project_root;
use crate::state::AppState;
use gwt_core::ai::{
    format_error_for_display, summarize_session, AIClient, AIError, AgentType as AiAgentType,
    ClaudeSessionParser, CodexSessionParser, GeminiSessionParser, OpenCodeSessionParser,
    SessionParseError, SessionParser, SessionSummary,
};
use gwt_core::config::{ProfilesConfig, ResolvedAISettings, ToolSessionEntry};
use serde::Serialize;
use std::path::Path;
use std::time::SystemTime;
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummaryUpdatedPayload {
    pub project_path: String,
    pub branch: String,
    pub result: SessionSummaryResult,
}

fn get_branch_session_summary_immediate(
    project_path: &str,
    branch: &str,
    state: &AppState,
) -> Result<(SessionSummaryResult, Option<SessionSummaryJob>), String> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let repo_key = repo_path.to_string_lossy().to_string();

    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Branch is required".to_string());
    }

    let entries = gwt_core::config::get_branch_tool_history(&repo_path, branch);
    let Some(entry) = entries.first() else {
        return Ok((summary_status("no-session", None, None, None), None));
    };

    let tool_id = entry.tool_id.trim().to_string();
    let session_id = entry.session_id.as_deref().unwrap_or("").trim().to_string();

    if tool_id.is_empty() || session_id.is_empty() {
        return Ok((
            summary_status(
                "no-session",
                if tool_id.is_empty() {
                    None
                } else {
                    Some(tool_id)
                },
                if session_id.is_empty() {
                    None
                } else {
                    Some(session_id)
                },
                None,
            ),
            None,
        ));
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

    Ok((immediate, Some(job)))
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
        start_session_summary_job(job, &state, app_handle);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::process::Command;
    use std::sync::Mutex;
    use tempfile::TempDir;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct TestEnvGuard {
        prev_home: Option<std::ffi::OsString>,
        prev_xdg: Option<std::ffi::OsString>,
    }

    impl TestEnvGuard {
        fn new(home_path: &Path) -> Self {
            let prev_home = std::env::var_os("HOME");
            let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
            std::env::set_var("HOME", home_path);
            std::env::set_var("XDG_CONFIG_HOME", home_path.join(".config"));
            Self {
                prev_home,
                prev_xdg,
            }
        }
    }

    impl Drop for TestEnvGuard {
        fn drop(&mut self) {
            match &self.prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.prev_xdg {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
        }
    }

    fn init_git_repo(path: &Path) {
        let out = Command::new("git")
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
            timestamp: 1,
        };

        gwt_core::config::save_session_entry(repo_root, entry).expect("save session entry");
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
        assert!(job.is_some());
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
            markdown: Some("## Purpose\nCached\n\n## Summary\nCached\n\n## Highlights\n- A\n".to_string()),
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
}
