use gwt_core::terminal::manager::PaneManager;
use std::path::PathBuf;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

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
    pub started_at_millis: i64,
}

pub struct AppState {
    pub project_path: Mutex<Option<String>>,
    pub pane_manager: Mutex<PaneManager>,
    pub agent_versions_cache: Mutex<HashMap<String, AgentVersionsCache>>,
    pub pane_launch_meta: Mutex<HashMap<String, PaneLaunchMeta>>,
    pub is_quitting: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            project_path: Mutex::new(None),
            pane_manager: Mutex::new(PaneManager::new()),
            agent_versions_cache: Mutex::new(HashMap::new()),
            pane_launch_meta: Mutex::new(HashMap::new()),
            is_quitting: AtomicBool::new(false),
        }
    }

    pub fn request_quit(&self) {
        self.is_quitting.store(true, Ordering::SeqCst);
    }
}
