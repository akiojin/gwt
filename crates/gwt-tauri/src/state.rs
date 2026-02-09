use gwt_core::ai::SessionSummaryCache;
use gwt_core::terminal::manager::PaneManager;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
    pub docker_service: Option<String>,
    pub docker_force_host: Option<bool>,
    pub docker_recreate: Option<bool>,
    pub docker_build: Option<bool>,
    pub docker_keep: Option<bool>,
    pub docker_container_name: Option<String>,
    pub started_at_millis: i64,
}

pub struct AppState {
    pub project_path: Mutex<Option<String>>,
    pub pane_manager: Mutex<PaneManager>,
    pub agent_versions_cache: Mutex<HashMap<String, AgentVersionsCache>>,
    pub session_summary_cache: Mutex<HashMap<String, SessionSummaryCache>>,
    pub session_summary_inflight: Mutex<HashSet<String>>,
    pub pane_launch_meta: Mutex<HashMap<String, PaneLaunchMeta>>,
    pub is_quitting: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            project_path: Mutex::new(None),
            pane_manager: Mutex::new(PaneManager::new()),
            agent_versions_cache: Mutex::new(HashMap::new()),
            session_summary_cache: Mutex::new(HashMap::new()),
            session_summary_inflight: Mutex::new(HashSet::new()),
            pane_launch_meta: Mutex::new(HashMap::new()),
            is_quitting: AtomicBool::new(false),
        }
    }

    pub fn request_quit(&self) {
        self.is_quitting.store(true, Ordering::SeqCst);
    }
}
