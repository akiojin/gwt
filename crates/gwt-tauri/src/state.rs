use crate::agent_master::AgentModeState;
use gwt_core::ai::SessionSummaryCache;
use gwt_core::config::os_env::EnvSource;
use gwt_core::terminal::manager::PaneManager;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::OnceCell;

#[derive(Debug, Clone)]
pub struct AgentVersionsCache {
    pub tags: Vec<String>,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PaneLaunchMeta {
    pub agent_id: String,
    pub branch: String,
    pub repo_path: PathBuf,
    pub worktree_path: PathBuf,
    pub tool_label: String,
    pub tool_version: String,
    pub mode: String,
    pub model: Option<String>,
    pub reasoning_level: Option<String>,
    pub skip_permissions: bool,
    pub collaboration_modes: bool,
    pub docker_service: Option<String>,
    pub docker_force_host: Option<bool>,
    pub docker_recreate: Option<bool>,
    pub docker_build: Option<bool>,
    pub docker_keep: Option<bool>,
    pub docker_container_name: Option<String>,
    pub docker_compose_args: Option<Vec<String>>,
    pub started_at_millis: i64,
}

#[derive(Debug, Clone)]
pub struct VersionHistoryCacheEntry {
    pub label: String,
    pub range_from: Option<String>,
    pub range_to: String,
    pub range_from_oid: Option<String>,
    pub range_to_oid: String,
    pub commit_count: u32,
    pub summary_markdown: String,
    pub changelog_markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTabMenuState {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowAgentTabsState {
    pub tabs: Vec<AgentTabMenuState>,
    pub active_tab_id: Option<String>,
}

pub struct AppState {
    /// Project root path per window label.
    ///
    /// Only stores windows that currently have a project opened.
    pub window_projects: Mutex<HashMap<String, String>>,
    /// One-shot permission to allow a window to actually close (instead of hiding to tray).
    ///
    /// Used to implement macOS Cmd+Q as "close the focused window" while keeping (x) as "hide".
    pub windows_allowed_to_close: Mutex<HashSet<String>>,
    /// Agent tab state per window label for native Window menu rendering.
    pub window_agent_tabs: Mutex<HashMap<String, WindowAgentTabsState>>,
    /// Agent mode conversation state per window label.
    pub window_agent_modes: Mutex<HashMap<String, AgentModeState>>,
    pub pane_manager: Mutex<PaneManager>,
    pub agent_versions_cache: Mutex<HashMap<String, AgentVersionsCache>>,
    pub session_summary_cache: Mutex<HashMap<String, SessionSummaryCache>>,
    pub session_summary_inflight: Mutex<HashSet<String>>,
    pub project_version_history_cache:
        Mutex<HashMap<String, HashMap<String, VersionHistoryCacheEntry>>>,
    pub project_version_history_inflight: Mutex<HashSet<String>>,
    pub pane_launch_meta: Mutex<HashMap<String, PaneLaunchMeta>>,
    /// Launch job cancellation flags keyed by job id.
    pub launch_jobs: Mutex<HashMap<String, Arc<AtomicBool>>>,
    pub is_quitting: AtomicBool,
    /// Prevent multiple exit confirmation dialogs from showing at once.
    pub exit_confirm_inflight: AtomicBool,
    pub os_env: Arc<OnceCell<HashMap<String, String>>>,
    pub os_env_source: Arc<OnceCell<EnvSource>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            window_projects: Mutex::new(HashMap::new()),
            windows_allowed_to_close: Mutex::new(HashSet::new()),
            window_agent_tabs: Mutex::new(HashMap::new()),
            window_agent_modes: Mutex::new(HashMap::new()),
            pane_manager: Mutex::new(PaneManager::new()),
            agent_versions_cache: Mutex::new(HashMap::new()),
            session_summary_cache: Mutex::new(HashMap::new()),
            session_summary_inflight: Mutex::new(HashSet::new()),
            project_version_history_cache: Mutex::new(HashMap::new()),
            project_version_history_inflight: Mutex::new(HashSet::new()),
            pane_launch_meta: Mutex::new(HashMap::new()),
            launch_jobs: Mutex::new(HashMap::new()),
            is_quitting: AtomicBool::new(false),
            exit_confirm_inflight: AtomicBool::new(false),
            os_env: Arc::new(OnceCell::new()),
            os_env_source: Arc::new(OnceCell::new()),
        }
    }

    /// Whether OS environment capture has completed.
    pub fn is_os_env_ready(&self) -> bool {
        self.os_env.initialized()
    }

    /// Wait briefly for OS environment capture to complete.
    ///
    /// This avoids non-deterministic launches when the UI requests a session before
    /// the startup capture task finishes.
    pub fn wait_os_env_ready(&self, timeout: Duration) -> bool {
        let start = std::time::Instant::now();
        while !self.is_os_env_ready() && start.elapsed() < timeout {
            std::thread::sleep(Duration::from_millis(50));
        }
        self.is_os_env_ready()
    }

    pub fn set_project_for_window(&self, window_label: &str, project_path: String) {
        if let Ok(mut map) = self.window_projects.lock() {
            map.insert(window_label.to_string(), project_path);
        }
    }

    pub fn clear_project_for_window(&self, window_label: &str) {
        if let Ok(mut map) = self.window_projects.lock() {
            map.remove(window_label);
        }
        if let Ok(mut map) = self.window_agent_tabs.lock() {
            map.remove(window_label);
        }
        if let Ok(mut map) = self.window_agent_modes.lock() {
            map.remove(window_label);
        }
    }

    pub fn project_for_window(&self, window_label: &str) -> Option<String> {
        let map = self.window_projects.lock().ok()?;
        map.get(window_label).cloned()
    }

    pub fn set_window_agent_tabs(
        &self,
        window_label: &str,
        tabs: Vec<AgentTabMenuState>,
        active_tab_id: Option<String>,
    ) {
        let normalized_active = active_tab_id.filter(|id| tabs.iter().any(|t| &t.id == id));
        if let Ok(mut map) = self.window_agent_tabs.lock() {
            map.insert(
                window_label.to_string(),
                WindowAgentTabsState {
                    tabs,
                    active_tab_id: normalized_active,
                },
            );
        }
    }

    pub fn window_agent_tabs_for_window(&self, window_label: &str) -> WindowAgentTabsState {
        let map = match self.window_agent_tabs.lock() {
            Ok(m) => m,
            Err(_) => return WindowAgentTabsState::default(),
        };
        map.get(window_label).cloned().unwrap_or_default()
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn allow_window_close(&self, window_label: &str) {
        if let Ok(mut set) = self.windows_allowed_to_close.lock() {
            set.insert(window_label.to_string());
        }
    }

    pub fn consume_window_close_permission(&self, window_label: &str) -> bool {
        if let Ok(mut set) = self.windows_allowed_to_close.lock() {
            return set.remove(window_label);
        }
        false
    }

    pub fn request_quit(&self) {
        self.is_quitting.store(true, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_projects_set_get_clear() {
        let state = AppState::new();
        assert_eq!(state.project_for_window("main"), None);

        state.set_project_for_window("main", "/tmp/repo".to_string());
        assert_eq!(
            state.project_for_window("main"),
            Some("/tmp/repo".to_string())
        );

        state.clear_project_for_window("main");
        assert_eq!(state.project_for_window("main"), None);
    }

    #[test]
    fn window_close_permission_is_one_shot() {
        let state = AppState::new();
        assert!(!state.consume_window_close_permission("main"));

        state.allow_window_close("main");
        assert!(state.consume_window_close_permission("main"));
        assert!(!state.consume_window_close_permission("main"));
    }

    #[test]
    fn window_agent_tabs_set_get_clear() {
        let state = AppState::new();
        assert_eq!(
            state.window_agent_tabs_for_window("main"),
            WindowAgentTabsState::default()
        );

        state.set_window_agent_tabs(
            "main",
            vec![
                AgentTabMenuState {
                    id: "agent-pane-1".to_string(),
                    label: "feature/one".to_string(),
                },
                AgentTabMenuState {
                    id: "agent-pane-2".to_string(),
                    label: "feature/two".to_string(),
                },
            ],
            Some("agent-pane-2".to_string()),
        );

        let tabs = state.window_agent_tabs_for_window("main");
        assert_eq!(tabs.tabs.len(), 2);
        assert_eq!(tabs.active_tab_id, Some("agent-pane-2".to_string()));

        state.clear_project_for_window("main");
        assert_eq!(
            state.window_agent_tabs_for_window("main"),
            WindowAgentTabsState::default()
        );
    }

    #[test]
    fn window_agent_tabs_active_is_cleared_when_missing() {
        let state = AppState::new();
        state.set_window_agent_tabs(
            "main",
            vec![AgentTabMenuState {
                id: "agent-pane-1".to_string(),
                label: "feature/one".to_string(),
            }],
            Some("agent-pane-999".to_string()),
        );

        let tabs = state.window_agent_tabs_for_window("main");
        assert_eq!(tabs.active_tab_id, None);
    }
}
