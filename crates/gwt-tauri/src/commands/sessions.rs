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
use tauri::State;

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

    Ok(gwt_core::config::get_branch_tool_history(&repo_path, branch))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummaryResult {
    pub status: String, // "ok" | "ai-not-configured" | "disabled" | "no-session" | "error"
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

fn session_parser_for_tool(tool_id: &str) -> Option<Box<dyn SessionParser>> {
    let agent_type = AiAgentType::from_tool_id(tool_id)?;
    match agent_type {
        AiAgentType::ClaudeCode => ClaudeSessionParser::with_default_home()
            .map(|p| Box::new(p) as Box<dyn SessionParser>),
        AiAgentType::CodexCli => CodexSessionParser::with_default_home()
            .map(|p| Box::new(p) as Box<dyn SessionParser>),
        AiAgentType::GeminiCli => GeminiSessionParser::with_default_home()
            .map(|p| Box::new(p) as Box<dyn SessionParser>),
        AiAgentType::OpenCode => OpenCodeSessionParser::with_default_home()
            .map(|p| Box::new(p) as Box<dyn SessionParser>),
    }
}

fn resolve_active_ai_settings(
    config: &ProfilesConfig,
) -> Option<(bool, bool, Option<ResolvedAISettings>)> {
    // Match the TUI behavior:
    // - Prefer active profile AI settings if present.
    // - Else fall back to default_ai.
    // - "ai_enabled": endpoint/model present
    // - "summary_enabled": ai_enabled && summary_enabled flag
    if let Some(profile) = config.active_profile() {
        if let Some(settings) = profile.ai.as_ref() {
            let ai_enabled = settings.is_enabled();
            let summary_enabled = settings.is_summary_enabled();
            let resolved = if ai_enabled {
                Some(settings.resolved())
            } else {
                None
            };
            return Some((ai_enabled, summary_enabled, resolved));
        }
    }

    let ai_enabled = config
        .default_ai
        .as_ref()
        .map(|s| s.is_enabled())
        .unwrap_or(false);
    let summary_enabled = config
        .default_ai
        .as_ref()
        .map(|s| s.is_summary_enabled())
        .unwrap_or(false);
    let resolved = config.default_ai.as_ref().map(|s| s.resolved());
    Some((ai_enabled, summary_enabled, resolved))
}

fn get_branch_session_summary_inner(
    project_path: &str,
    branch: &str,
    state: &AppState,
) -> Result<SessionSummaryResult, String> {
    let project_root = Path::new(project_path);
    let repo_path = resolve_repo_path_for_project_root(project_root)?;
    let repo_key = repo_path.to_string_lossy().to_string();

    let branch = branch.trim();
    if branch.is_empty() {
        return Err("Branch is required".to_string());
    }

    let entries = gwt_core::config::get_branch_tool_history(&repo_path, branch);
    let Some(entry) = entries.first() else {
        return Ok(summary_status("no-session", None, None, None));
    };

    let tool_id = entry.tool_id.trim().to_string();
    let session_id = entry
        .session_id
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_string();

    if tool_id.is_empty() || session_id.is_empty() {
        return Ok(summary_status(
            "no-session",
            if tool_id.is_empty() { None } else { Some(tool_id) },
            if session_id.is_empty() { None } else { Some(session_id) },
            None,
        ));
    }

    let profiles = ProfilesConfig::load().map_err(|e| e.to_string())?;
    let (ai_enabled, summary_enabled, resolved) =
        resolve_active_ai_settings(&profiles).unwrap_or((false, false, None));

    if !ai_enabled {
        return Ok(summary_status(
            "ai-not-configured",
            Some(tool_id),
            Some(session_id),
            None,
        ));
    }
    if !summary_enabled {
        return Ok(summary_status(
            "disabled",
            Some(tool_id),
            Some(session_id),
            None,
        ));
    }

    let settings = resolved.ok_or_else(|| "AI settings are not configured".to_string())?;
    let parser = match session_parser_for_tool(&tool_id) {
        Some(p) => p,
        None => {
            return Ok(summary_status(
                "error",
                Some(tool_id),
                Some(session_id),
                Some("Unsupported agent session".to_string()),
            ))
        }
    };

    let path = parser.session_file_path(&session_id);
    let metadata = match std::fs::metadata(&path) {
        Ok(meta) => meta,
        Err(err) => {
            let missing = err.kind() == std::io::ErrorKind::NotFound;
            return Ok(summary_status(
                if missing { "no-session" } else { "error" },
                Some(tool_id),
                Some(session_id),
                Some(err.to_string()),
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
        return Ok(ok_summary(&tool_id, &session_id, summary));
    }

    let client = AIClient::new(settings).map_err(|e| e.to_string())?;

    let parsed = match parser.parse(&session_id) {
        Ok(parsed) => parsed,
        Err(err) => {
            let missing = matches!(err, SessionParseError::FileNotFound(_));
            return Ok(summary_status(
                if missing { "no-session" } else { "error" },
                Some(tool_id),
                Some(session_id),
                Some(err.to_string()),
            ));
        }
    };

    match summarize_session(&client, &parsed) {
        Ok(summary) => {
            {
                let mut cache_guard = state
                    .session_summary_cache
                    .lock()
                    .map_err(|_| "Session summary cache lock poisoned".to_string())?;
                cache_guard
                    .entry(repo_key)
                    .or_default()
                    .set(branch.to_string(), session_id.clone(), summary.clone(), mtime);
            }
            Ok(ok_summary(&tool_id, &session_id, &summary))
        }
        Err(AIError::IncompleteSummary) => {
            if let Some(prev) = previous_any.as_ref() {
                let mut out = ok_summary(&tool_id, &session_id, prev);
                out.warning = Some("Incomplete summary; keeping previous".to_string());
                Ok(out)
            } else {
                Ok(summary_status(
                    "error",
                    Some(tool_id),
                    Some(session_id),
                    Some(format_error_for_display(&AIError::IncompleteSummary)),
                ))
            }
        }
        Err(other) => Ok(summary_status(
            "error",
            Some(tool_id),
            Some(session_id),
            Some(format_error_for_display(&other)),
        )),
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
) -> Result<SessionSummaryResult, String> {
    get_branch_session_summary_inner(&project_path, &branch, &state)
}

#[cfg(test)]
mod tests {
    use super::*;
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
            Self { prev_home, prev_xdg }
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
        let out = Command::new("git").args(["init"]).current_dir(path).output();
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
        let out =
            get_branch_session_summary_inner(repo.path().to_str().unwrap(), "main", &state).unwrap();
        assert_eq!(out.status, "no-session");
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
        let out =
            get_branch_session_summary_inner(repo.path().to_str().unwrap(), "main", &state).unwrap();
        assert_eq!(out.status, "ai-not-configured");
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
        let out =
            get_branch_session_summary_inner(repo.path().to_str().unwrap(), "main", &state).unwrap();
        assert_eq!(out.status, "disabled");
        assert_eq!(out.tool_id.as_deref(), Some("codex-cli"));
        assert_eq!(out.session_id.as_deref(), Some("session-1"));
    }
}

