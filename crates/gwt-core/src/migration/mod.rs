//! Migration of legacy Normal Git repositories into the Nested Bare+Worktree
//! layout used by gwt (`<workspace>/<repo>.git/` + `<workspace>/<branch>/`).
//!
//! Entry point: [`executor::execute_migration`].

pub mod backup;
pub mod executor;
pub mod rollback;
pub mod types;
pub mod validator;

pub use types::{
    MigrationError, MigrationOptions, MigrationOutcome, MigrationPhase, MigrationPlan,
    RecoveryState, WorktreeMigration,
};
