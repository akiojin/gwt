//! gwt - Git Worktree Manager CLI

use clap::Parser;
use gwt_core::error::GwtError;

mod cli;
mod tui;

use cli::{Cli, Commands};

fn main() {
    if let Err(e) = run() {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<(), GwtError> {
    let cli = Cli::parse();

    // Check if git is available
    if !check_git_available() {
        return Err(GwtError::GitNotFound);
    }

    // Initialize logging
    let log_config = gwt_core::logging::LogConfig {
        debug: cli.debug || std::env::var("GWT_DEBUG").is_ok(),
        ..Default::default()
    };
    gwt_core::logging::init_logger(&log_config)?;

    match cli.command {
        Some(Commands::Serve { port }) => {
            println!("Starting server on port {}...", port);
            // TODO: Start web server
            Ok(())
        }
        None => {
            // Interactive TUI mode
            tui::run()
        }
    }
}

fn check_git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok()
}
