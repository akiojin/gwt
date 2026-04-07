//! gwt-core: Thin foundational crate for the gwt ecosystem.
//!
//! Provides shared error types, filesystem path utilities, and process
//! execution helpers. No business logic lives here — domain crates
//! (gwt-git, gwt-agent, etc.) build on top of these primitives.

pub mod error;
pub mod index;
pub mod paths;
pub mod process;
pub mod repo_hash;
pub mod runtime;
pub mod worktree_hash;

pub use error::{GwtError, Result};
