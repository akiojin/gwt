//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Built with the Elm Architecture (Model / View / Update) pattern.
#![allow(dead_code)]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::current_dir().unwrap_or_default();
    gwt_tui::app::run(repo_root)
}
