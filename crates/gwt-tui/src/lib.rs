//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Elm Architecture: Model -> Message -> Update -> View
//! Built with ratatui + crossterm.

pub mod app;
pub(crate) mod custom_agents;
pub mod event;
pub mod input;
pub mod message;
pub mod model;
pub mod notification_router;
pub mod renderer;
pub mod screens;
pub mod theme;
pub mod widgets;

pub use gwt_clipboard::clipboard_payload_to_bytes;

#[cfg(test)]
pub(crate) static DOCKER_ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
pub(crate) static GH_PATH_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
