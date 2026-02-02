//! Migration module for converting .worktrees/ method to bare method (SPEC-a70a1ece US7-US9)
//!
//! This module provides functionality to migrate existing repositories using the
//! `.worktrees/` subdirectory method to the bare repository + sibling worktree method.

mod backup;
mod config;
mod error;
mod executor;
mod rollback;
mod state;
mod validator;

pub use backup::{create_backup, restore_backup, BackupInfo};
pub use config::MigrationConfig;
pub use error::MigrationError;
pub use executor::{
    derive_bare_repo_name, execute_migration, MigrationProgress, WorktreeMigrationInfo,
};
pub use rollback::rollback_migration;
pub use state::MigrationState;
pub use validator::{validate_migration, ValidationResult};
