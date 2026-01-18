//! gwt-core: Core library for Git Worktree Manager
//!
//! This crate provides the core functionality for managing Git worktrees,
//! including Git operations, configuration management, logging, file locking,
//! and AI agent integration.

pub mod agent;
pub mod config;
pub mod error;
pub mod execution_mode;
pub mod git;
pub mod lock;
pub mod logging;
pub mod tmux;
pub mod worktree;

pub use error::{GwtError, Result};
pub use execution_mode::ExecutionMode;
