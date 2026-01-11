//! gwt-core: Core library for Git Worktree Manager
//!
//! This crate provides the core functionality for managing Git worktrees,
//! including Git operations, configuration management, logging, and file locking.

pub mod error;
pub mod git;
pub mod worktree;
pub mod config;
pub mod logging;
pub mod lock;

pub use error::{GwtError, Result};
