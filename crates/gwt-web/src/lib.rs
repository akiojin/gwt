//! gwt-web: Web server for Git Worktree Manager

pub mod api;
pub mod server;
pub mod static_files;
pub mod websocket;

pub use api::AppState;
pub use server::{serve, serve_with_config, ServerConfig};
