//! gwt-tui: TUI frontend for Git Worktree Manager
#![allow(dead_code, clippy::should_implement_trait, clippy::len_zero)]

pub mod app;
pub mod config;
pub mod event;
pub mod input;
pub mod message;
pub mod model;
pub mod renderer;
pub mod screens;
pub mod widgets;

// Re-export wizard types for library consumers
pub use screens::wizard;
