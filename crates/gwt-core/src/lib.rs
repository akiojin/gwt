//! gwt-core: Thin foundational crate for the gwt ecosystem.
//!
//! Provides shared error types, filesystem path utilities, and process
//! execution helpers. No business logic lives here — domain crates
//! (gwt-git, gwt-agent, etc.) build on top of these primitives.

pub mod config;
pub mod coordination;
pub mod daemon;
pub mod error;
pub mod index;
pub mod logging;
pub mod migration;
pub mod notes;
pub mod paths;
pub mod process;
mod release_contract;
pub mod repo_hash;
pub mod runtime;
pub mod skill_state;
#[cfg(test)]
pub(crate) mod test_support;
pub mod update;
pub mod workspace_projection;
pub mod worktree_hash;

pub use error::{GwtError, Result};
