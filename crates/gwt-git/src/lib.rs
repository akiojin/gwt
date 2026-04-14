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

pub use branch::{
    delete_local_branch, detect_cleanable_target, git_divergence, is_branch_merged_into,
    is_protected_branch, list_gone_branches, Branch, DivergenceInfo, MergeTarget,
};
pub use commit::CommitEntry;
pub use diff::{FileEntry, FileStatus};
pub use issue::{Issue, IssueCache};
pub use pr_status::{
    fetch_pr_list, pr_check_report, CiStatus, MergeStatus, PrCheckReport, PrStatus, ReviewStatus,
};
pub use repository::{
    clone_repo, detect_repo_type, initialize_workspace, install_develop_protection, RepoType,
    Repository,
};
pub use worktree::{sibling_worktree_path, RemoteDeleteOutcome, WorktreeInfo, WorktreeManager};
