//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Built with the Elm Architecture (Model / View / Update) pattern.

mod app;
mod config;
mod event;
mod input;
mod message;
mod model;
mod renderer;
mod screens;
mod widgets;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let repo_root = std::env::current_dir().unwrap_or_default();
    app::run(repo_root)
}
