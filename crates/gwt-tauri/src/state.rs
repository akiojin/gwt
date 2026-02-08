use gwt_core::terminal::manager::PaneManager;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

pub struct AppState {
    pub project_path: Mutex<Option<String>>,
    pub pane_manager: Mutex<PaneManager>,
    pub is_quitting: AtomicBool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            project_path: Mutex::new(None),
            pane_manager: Mutex::new(PaneManager::new()),
            is_quitting: AtomicBool::new(false),
        }
    }

    pub fn request_quit(&self) {
        self.is_quitting.store(true, Ordering::SeqCst);
    }
}
