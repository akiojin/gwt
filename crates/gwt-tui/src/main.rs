//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Built with the Elm Architecture (Model / View / Update) pattern.
<<<<<<< HEAD

#![allow(dead_code)]

mod app;
mod config;
mod event;
mod input;
mod message;
mod model;
mod renderer;
mod screens;
mod widgets;
=======
#![allow(dead_code)]
>>>>>>> origin/feature/feature-1776

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let log_config = gwt_core::logging::LogConfig::default();
    let _profiling_guard = gwt_core::logging::init_logger(&log_config).ok();

    let repo_root = std::env::current_dir().unwrap_or_default();
<<<<<<< HEAD

    // Skill registration (FR-073)
    if let Ok(settings) = gwt_core::config::Settings::load_global() {
        let _ = gwt_core::config::repair_skill_registration_with_settings_at_project_root(
            &settings,
            Some(repo_root.as_path()),
        );
    }

    app::run(repo_root)
=======
    gwt_tui::app::run(repo_root)
>>>>>>> origin/feature/feature-1776
}
