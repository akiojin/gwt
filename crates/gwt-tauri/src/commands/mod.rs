pub mod agents;
pub mod branch_suggest;
pub mod branches;
pub mod docker;
pub mod profiles;
pub mod project;
pub mod sessions;
pub mod settings;
pub mod terminal;

#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to gwt.", name)
}
