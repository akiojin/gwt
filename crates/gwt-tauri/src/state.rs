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
    pub started_at_millis: i64,
}

pub struct AppState {
    /// Project root path per window label.
    ///
    /// Only stores windows that currently have a project opened.
    pub window_projects: Mutex<HashMap<String, String>>,
    pub pane_manager: Mutex<PaneManager>,
    pub agent_versions_cache: Mutex<HashMap<String, AgentVersionsCache>>,
    pub session_summary_cache: Mutex<HashMap<String, SessionSummaryCache>>,
    pub session_summary_inflight: Mutex<HashSet<String>>,
    pub pane_launch_meta: Mutex<HashMap<String, PaneLaunchMeta>>,
    pub is_quitting: AtomicBool,
    pub os_env: Arc<OnceCell<HashMap<String, String>>>,
    pub os_env_source: Arc<OnceCell<EnvSource>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            window_projects: Mutex::new(HashMap::new()),
            pane_manager: Mutex::new(PaneManager::new()),
            agent_versions_cache: Mutex::new(HashMap::new()),
            session_summary_cache: Mutex::new(HashMap::new()),
            session_summary_inflight: Mutex::new(HashSet::new()),
            pane_launch_meta: Mutex::new(HashMap::new()),
            is_quitting: AtomicBool::new(false),
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
    }

    pub fn project_for_window(&self, window_label: &str) -> Option<String> {
        let map = self.window_projects.lock().ok()?;
        map.get(window_label).cloned()
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
}
