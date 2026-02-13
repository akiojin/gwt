//! gwt-core: Core library for Git Worktree Manager
//!
//! This crate provides the core functionality for managing Git worktrees,
//! including Git operations, configuration management, logging, file locking,
//! and AI agent integration.

pub mod agent;
pub mod ai;
pub mod config;
pub mod docker;
pub mod error;
pub mod git;
pub mod lock;
pub mod logging;
pub mod migration;
pub mod process;
pub mod terminal;
pub mod worktree;

pub use error::{GwtError, Result};
