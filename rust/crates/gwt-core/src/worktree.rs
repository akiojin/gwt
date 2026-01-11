//! Worktree management module
//!
//! Provides Git worktree creation, deletion, and management functionality.

mod manager;
mod path;
mod types;

pub use manager::WorktreeManager;
pub use path::WorktreePath;
pub use types::{CleanupCandidate, Worktree, WorktreeStatus};
