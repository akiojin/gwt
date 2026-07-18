//! gwt-core: Thin foundational crate for the gwt ecosystem.
//!
//! Provides shared error types, filesystem path utilities, and process
//! execution helpers. No business logic lives here — domain crates
//! (gwt-git, gwt-agent, etc.) build on top of these primitives.

pub mod board_remote_roots;
pub mod config;
pub mod coordination;
pub mod daemon;
pub mod error;
pub mod index;
pub mod index_coordinator;
pub mod logging;
pub mod migration;
pub mod paths;
pub mod process;
pub mod process_console;
pub mod process_executor;
mod release_contract;
pub mod release_notes;
pub mod repo_hash;
pub mod runtime;
pub mod skill_state;
#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
pub mod update;
pub mod usage;
pub mod work_events_intake;
pub mod workspace_projection;
pub mod workspace_projection_migration;
pub mod worktree_hash;

pub use error::{GwtError, JsonDecodeKind, Result};

#[cfg(test)]
mod canonical_naming_tests {
    //! SPEC-2359 US-66 (user decision 2026-06-12): `WorkspaceProjection` in
    //! the `workspace_projection` module is the canonical current-state type
    //! (the Workspace is the branch-level place). The per-launch record
    //! family keeps its Work-era aliases until the terminology settles.

    #[test]
    fn workspace_projection_is_canonical() {
        let projection = crate::workspace_projection::WorkspaceProjection::default_for_project(
            std::path::Path::new("/tmp/repo"),
        );
        let _ = projection;
    }
}
