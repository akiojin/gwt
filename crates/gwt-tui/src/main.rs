//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Built with the Elm Architecture (Model / View / Update) pattern.
#![allow(dead_code)]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let log_config = gwt_core::logging::LogConfig::default();
    let _profiling_guard = gwt_core::logging::init_logger(&log_config).ok();

    let repo_root = std::env::current_dir().unwrap_or_default();

    // Note: Skill registration (FR-073) is deferred to agent launch time,
    // not at gwt-tui startup. Running it at startup against the CWD can
    // delete files needed for compilation (memory/constitution.md).

    gwt_tui::app::run(repo_root)
}
