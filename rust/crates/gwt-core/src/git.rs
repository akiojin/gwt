//! Git operations module
//!
//! Provides Git repository operations using gitoxide (gix) with fallback to external git commands.

mod backend;
mod branch;
mod remote;
mod repository;

pub use backend::GitBackend;
pub use branch::Branch;
pub use remote::Remote;
pub use repository::Repository;
