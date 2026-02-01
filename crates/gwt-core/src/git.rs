//! Git operations module
//!
//! Provides Git repository operations using gitoxide (gix) with fallback to external git commands.

mod backend;
mod branch;
mod clone;
mod commit;
mod issue;
mod pullrequest;
mod remote;
mod repository;
mod submodule;

pub use backend::GitBackend;
pub use branch::{Branch, DivergenceStatus};
pub use clone::{clone_bare, extract_repo_name, CloneConfig};
pub use commit::{
    BranchMeta, BranchSummary, ChangeStats, CommitEntry, LoadingState, SectionErrors,
};
pub use issue::{
    create_linked_branch, fetch_open_issues, filter_issues_by_title, find_branch_for_issue,
    generate_branch_name, is_gh_cli_available, parse_gh_issues_json, GitHubIssue,
};
pub use pullrequest::{PrCache, PullRequest};
pub use remote::Remote;
pub use repository::{
    detect_repo_type, get_header_context, get_main_repo_root, is_bare_repository, is_empty_dir,
    is_git_repo, is_inside_worktree, HeaderContext, RepoType, Repository, WorktreeInfo,
};
pub use submodule::{has_submodules, init_submodules, list_submodules};
