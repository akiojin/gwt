use gwt_core::terminal::manager::PaneManager;
use std::sync::Mutex;

pub struct AppState {
    pub project_path: Mutex<Option<String>>,
    pub pane_manager: Mutex<PaneManager>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            project_path: Mutex::new(None),
            pane_manager: Mutex::new(PaneManager::new()),
        }
    }
}
