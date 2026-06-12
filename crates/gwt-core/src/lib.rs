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
pub mod work_projection;
/// SPEC-2359 US-66 (T-526): legacy adapter module — the canonical module is
/// [`work_projection`]. Re-exports everything so staged migration never
/// breaks call sites; new code must use the Work spelling.
pub mod workspace_projection {
    pub use crate::work_projection::*;
}
pub mod workspace_projection_migration;
pub mod worktree_hash;

pub use error::{GwtError, Result};

#[cfg(test)]
mod canonical_naming_tests {
    //! SPEC-2359 US-66 (T-525): canonical names are Work-based; the legacy
    //! `workspace_projection` module / `WorkspaceProjection` type survive
    //! only as adapter aliases.

    #[test]
    fn work_projection_is_canonical_and_legacy_names_are_aliases() {
        // Canonical module + type.
        let canonical = crate::work_projection::WorkProjection::default_for_project(
            std::path::Path::new("/tmp/repo"),
        );
        // Legacy adapter spellings resolve to the SAME type.
        let legacy: crate::workspace_projection::WorkspaceProjection = canonical;
        let _ = legacy;
    }
}
