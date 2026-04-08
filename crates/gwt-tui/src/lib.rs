//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Elm Architecture: Model -> Message -> Update -> View
//! Built with ratatui + crossterm.

pub mod app;
pub(crate) mod custom_agents;
pub mod event;
pub mod index_worker;
pub mod input;
pub mod logs_watcher;
pub mod message;
pub mod model;
pub mod notification_router;
pub mod renderer;
pub mod screens;
pub(crate) mod scroll_debug;
pub mod theme;
pub mod widgets;

#[cfg(test)]
pub(crate) static DOCKER_ENV_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
#[cfg(test)]
pub(crate) static GH_PATH_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
