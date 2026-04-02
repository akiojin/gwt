//! gwt-git: Git operations library for gwt
//!
//! Provides repository discovery, branch listing, worktree management,
//! GitHub Issue/PR tracking, diff helpers, and commit log queries.

pub mod branch;
pub mod commit;
pub mod diff;
pub mod issue;
pub mod pr_status;
pub mod repository;
pub mod worktree;

pub use branch::Branch;
pub use commit::CommitEntry;
pub use diff::{FileEntry, FileStatus};
pub use issue::{Issue, IssueCache};
pub use pr_status::{
    pr_check_report, CiStatus, MergeStatus, PrCheckReport, PrStatus, ReviewStatus,
};
pub use repository::{
    clone_repo, detect_repo_type, initialize_workspace, install_develop_protection, RepoType,
    Repository,
};
pub use worktree::{WorktreeInfo, WorktreeManager};
