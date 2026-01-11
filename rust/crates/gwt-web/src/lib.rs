//! gwt-web: Web server for Git Worktree Manager

pub mod api;
pub mod server;
pub mod websocket;

pub use server::serve;
