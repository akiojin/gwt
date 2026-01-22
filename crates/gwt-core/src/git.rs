//! Git operations module
//!
//! Provides Git repository operations using gitoxide (gix) with fallback to external git commands.

mod backend;
mod branch;
mod commit;
mod pullrequest;
mod remote;
mod repository;

pub use backend::GitBackend;
pub use branch::{Branch, DivergenceStatus};
pub use commit::{
    BranchMeta, BranchSummary, ChangeStats, CommitEntry, LoadingState, SectionErrors,
};
pub use pullrequest::{PrCache, PullRequest};
pub use remote::Remote;
pub use repository::{get_main_repo_root, Repository, WorktreeInfo};
