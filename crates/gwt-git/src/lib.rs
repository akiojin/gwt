//! gwt-git: Git operations library for gwt
//!
//! Provides repository discovery, branch listing, worktree management,
//! GitHub Issue/PR tracking, diff helpers, and commit log queries.

pub mod blob;
pub mod branch;
pub mod branch_protection;
pub mod commit;
pub mod diff;
pub mod gh_rest;
pub mod issue;
pub mod merge_conflict;
pub mod merged_branch_prune;
pub mod merged_pr_sync;
pub mod migration;
pub mod pr_status;
pub mod refs;
pub mod release_status;
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
pub use merge_conflict::{
    measure_pr_conflict, remote_tracking_ref, PrConflictReport, CONFLICT_FILE_LIST_CAP,
};
pub use pr_status::{
    check_counts_from_rollup, classify_pr_lifecycle, classify_pr_lifecycle_with,
    classify_unlanded_branches, collect_unlanded_work_branches, fetch_pr_inventory_tracked,
    fetch_pr_list, parse_pr_inventory_json, parse_pr_inventory_json_with,
    parse_unlanded_branch_refs, pr_check_report, CiStatus, MergeStatus, PrCheckCounts,
    PrCheckReport, PrClosingIssue, PrInventoryFields, PrInventoryHistory, PrInventoryHistoryEntry,
    PrInventoryInclude, PrInventoryItem, PrInventoryOptions, PrInventoryRead, PrLifecycleClass,
    PrLifecycleDecision, PrStatus, ReviewStatus, UnlandedBranch, UnlandedBranchProbe,
    PR_ESCALATE_AFTER_UNCHANGED_CYCLES, PR_FALLBACK_WHEN_NOT_EXECUTABLE, PR_INVENTORY_CACHE_FILE,
    PR_INVENTORY_CACHE_TTL_SECS, PR_INVENTORY_HISTORY_FILE, PR_STALE_AFTER_HOURS,
    PR_VIEW_JSON_FIELDS, UNLANDED_BRANCH_BASE_REF,
};
pub use refs::{list_existing_refs, resolve_canonical_root_tree, CanonicalRootTree};
pub use repository::{
    clone_project_as_nested_bare, clone_repo, derive_github_project_clone_target, detect_repo_type,
    initialize_workspace, install_develop_protection, GitHubProjectCloneOutcome,
    GitHubProjectCloneTarget, RepoType, Repository,
};
pub use worktree::{sibling_worktree_path, RemoteDeleteOutcome, WorktreeInfo, WorktreeManager};
