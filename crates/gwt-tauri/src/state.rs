use gwt_core::terminal::manager::PaneManager;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct AgentVersionsCache {
    pub tags: Vec<String>,
    pub versions: Vec<String>,
}

pub struct AppState {
    pub project_path: Mutex<Option<String>>,
    pub pane_manager: Mutex<PaneManager>,
    pub agent_versions_cache: Mutex<HashMap<String, AgentVersionsCache>>,
    pub is_quitting: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            project_path: Mutex::new(None),
            pane_manager: Mutex::new(PaneManager::new()),
            agent_versions_cache: Mutex::new(HashMap::new()),
            is_quitting: AtomicBool::new(false),
        }
    }

    pub fn request_quit(&self) {
        self.is_quitting.store(true, Ordering::SeqCst);
    }
}
