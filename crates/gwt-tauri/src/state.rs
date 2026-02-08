use std::sync::Mutex;

#[allow(dead_code)]
pub struct AppState {
    pub project_path: Mutex<Option<String>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            project_path: Mutex::new(None),
        }
    }
}
