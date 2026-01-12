//! File locking module
//!
//! Provides per-worktree file locking for multi-instance support using flock.

mod guard;
mod worktree_lock;

pub use guard::LockGuard;
pub use worktree_lock::WorktreeLock;
