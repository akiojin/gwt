//! Git operations module
//!
//! Provides Git repository operations using gitoxide (gix) with fallback to external git commands.

mod branch;
mod clone;
mod commit;
pub mod diff;
mod gh_cli;
mod issue;
mod pullrequest;
mod remote;
mod repository;
pub mod stash;
mod submodule;

pub use branch::{Branch, DivergenceStatus};
pub use clone::{clone_bare, extract_repo_name, CloneConfig};
pub use commit::{
    BranchMeta, BranchSummary, ChangeStats, CommitEntry, LoadingState, SectionErrors,
};
pub use diff::{
    detect_base_branch, get_branch_commits, get_branch_diff_files, get_file_diff,
    get_git_change_summary, get_working_tree_status, list_base_branch_candidates, FileChange,
    FileChangeKind, FileDiff, GitChangeSummary, GitViewCommit, WorkingTreeEntry,
};
pub use issue::{
    create_linked_branch, fetch_open_issues, filter_issues_by_title, find_branch_for_issue,
    generate_branch_name, is_gh_cli_authenticated, is_gh_cli_available, parse_gh_issues_json,
    FetchIssuesResult, GitHubIssue,
};
pub use pullrequest::{PrCache, PullRequest};
pub use remote::Remote;
pub use repository::{
    detect_repo_type, find_bare_repo_in_dir, get_header_context, get_main_repo_root,
    is_bare_repository, is_empty_dir, is_git_repo, is_inside_worktree, HeaderContext, RepoType,
    Repository, WorktreeInfo,
};
pub use stash::{get_stash_list, StashEntry};
pub use submodule::{has_submodules, init_submodules, list_submodules};
