//! Migration orchestrator (SPEC-1934 US-6).
//!
//! Lives in the `gwt` crate (not `gwt-core`) so it can depend on both
//! `gwt-core` (validator/backup/rollback/types) and `gwt-git` (pure Git
//! operations) without introducing a cycle.

pub mod executor;

pub use executor::execute_migration;
