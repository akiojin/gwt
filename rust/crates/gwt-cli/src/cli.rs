//! CLI argument parsing

use clap::{Parser, Subcommand};

/// Git Worktree Manager - A TUI for managing Git worktrees
#[derive(Parser, Debug)]
#[command(name = "gwt")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Enable debug logging
    #[arg(short, long, env = "GWT_DEBUG")]
    pub debug: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Start the web UI server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,
    },
}
