//! CLI argument parsing

use clap::{Parser, Subcommand, ValueEnum};

/// Git Worktree Manager - A TUI for managing Git worktrees
#[derive(Parser, Debug)]
#[command(name = "gwt")]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Enable debug logging
    #[arg(short, long, env = "GWT_DEBUG")]
    pub debug: bool,

    /// Repository root path (default: auto-detect)
    #[arg(short = 'C', long, env = "GWT_REPO")]
    pub repo: Option<std::path::PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// List all worktrees
    #[command(alias = "ls")]
    List {
        /// Output format
        #[arg(short, long, default_value = "table")]
        format: OutputFormat,
    },

    /// Add a new worktree
    Add {
        /// Branch name for the worktree
        branch: String,

        /// Create new branch (use existing if not set)
        #[arg(short, long)]
        new: bool,

        /// Base branch for new worktree
        #[arg(short, long)]
        base: Option<String>,
    },

    /// Remove a worktree
    #[command(alias = "rm")]
    Remove {
        /// Branch name or path of worktree to remove
        target: String,

        /// Force removal even with uncommitted changes
        #[arg(short, long)]
        force: bool,

        /// Also delete the branch
        #[arg(long)]
        delete_branch: bool,
    },

    /// Switch to a worktree
    #[command(alias = "sw")]
    Switch {
        /// Branch name to switch to
        branch: String,

        /// Open in new terminal window
        #[arg(short, long)]
        new_window: bool,
    },

    /// Clean up orphaned worktrees
    Clean {
        /// Dry run (show what would be cleaned)
        #[arg(short, long)]
        dry_run: bool,

        /// Also prune git metadata
        #[arg(short, long)]
        prune: bool,
    },

    /// View logs
    Logs {
        /// Number of log entries to show
        #[arg(short, long, default_value = "50")]
        limit: usize,

        /// Follow log output
        #[arg(short, long)]
        follow: bool,
    },

    /// Start the web UI server
    Serve {
        /// Port to listen on
        #[arg(short, long, default_value = "3000")]
        port: u16,

        /// Bind address
        #[arg(short, long, default_value = "127.0.0.1")]
        address: String,
    },

    /// Initialize gwt configuration
    Init {
        /// Force overwrite existing config
        #[arg(short, long)]
        force: bool,
    },

    /// Lock a worktree
    Lock {
        /// Branch name or path of worktree to lock
        target: String,

        /// Lock reason
        #[arg(short, long)]
        reason: Option<String>,
    },

    /// Unlock a worktree
    Unlock {
        /// Branch name or path of worktree to unlock
        target: String,
    },

    /// Repair worktree metadata
    Repair {
        /// Specific worktree to repair (repairs all if not specified)
        target: Option<String>,
    },
}

/// Output format for list command
#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum OutputFormat {
    /// Table format (default)
    #[default]
    Table,
    /// JSON format
    Json,
    /// Simple format (one per line)
    Simple,
}
