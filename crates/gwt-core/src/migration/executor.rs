//! Orchestrates the migration phases (Validate → Backup → Bareify →
//! Worktrees → Submodules → Tracking → Cleanup).
//!
//! The implementation is filled in by Phase 5+ tasks in
//! `tasks.md`; the skeleton here only exposes the public entry point so
//! downstream modules and tests can compile.

use std::path::Path;

use super::types::{
    MigrationError, MigrationOptions, MigrationOutcome, MigrationPhase, RecoveryState,
};

/// Public entry point. Drives the full Normal→Bare+Worktree migration.
///
/// `progress` receives `(phase, percent_complete_within_phase)` updates so a
/// caller can stream progress to the WebSocket UI.
pub fn execute_migration(
    _project_root: &Path,
    _options: MigrationOptions,
    _progress: impl FnMut(MigrationPhase, u8),
) -> Result<MigrationOutcome, MigrationError> {
    Err(MigrationError {
        phase: MigrationPhase::Confirm,
        message: "execute_migration is not implemented yet (SPEC-1934 US-6)".to_string(),
        recovery: RecoveryState::Untouched,
    })
}
