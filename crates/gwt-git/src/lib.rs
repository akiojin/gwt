//! gwt-git: Git operations library for gwt
//!
//! Provides repository discovery, branch listing, worktree management,
//! GitHub Issue/PR tracking, diff helpers, and commit log queries.

pub mod blob;
pub mod branch;
pub mod branch_protection;
pub mod commit;
pub mod diff;
pub mod issue;
pub mod migration;
pub mod pr_status;
pub mod refs;
pub mod repository;
pub mod worktree;

pub use branch::{
    delete_local_branch, detect_cleanable_target, detect_cleanable_target_with_remote_names,
    git_divergence, is_branch_merged_into, is_protected_branch, list_gone_branches,
    list_remote_names, Branch, DivergenceInfo, MergeTarget, MergeTargetRef,
};
pub use commit::CommitEntry;
pub use diff::{FileEntry, FileStatus};
pub use issue::{Issue, IssueCache};
pub use pr_status::{
    classify_pr_lifecycle, classify_pr_lifecycle_with, fetch_pr_inventory,
    fetch_pr_inventory_tracked, fetch_pr_list, parse_pr_inventory_json,
    parse_pr_inventory_json_with, pr_check_report, CiStatus, MergeStatus, PrCheckReport,
    PrClosingIssue, PrInventoryFields, PrInventoryHistory, PrInventoryHistoryEntry,
    PrInventoryItem, PrInventoryOptions, PrLifecycleClass, PrLifecycleDecision, PrStatus,
    ReviewStatus, PR_ESCALATE_AFTER_UNCHANGED_CYCLES, PR_FALLBACK_WHEN_NOT_EXECUTABLE,
    PR_INVENTORY_HISTORY_FILE, PR_STALE_AFTER_HOURS,
};
pub use refs::list_existing_refs;
pub use repository::{
    clone_project_as_nested_bare, clone_repo, derive_github_project_clone_target, detect_repo_type,
    initialize_workspace, install_develop_protection, GitHubProjectCloneOutcome,
    GitHubProjectCloneTarget, RepoType, Repository,
};
pub use worktree::{sibling_worktree_path, RemoteDeleteOutcome, WorktreeInfo, WorktreeManager};
