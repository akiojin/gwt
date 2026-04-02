//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Elm Architecture: Model -> Message -> Update -> View
//! Built with ratatui + crossterm.

pub mod app;
pub mod event;
pub mod input;
pub mod message;
pub mod model;
pub mod notification_router;
pub mod renderer;
pub mod screens;
pub mod widgets;
