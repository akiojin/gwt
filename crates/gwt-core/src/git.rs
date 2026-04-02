//! Git operations module
//!
//! Provides Git repository operations using gitoxide (gix) with fallback to external git commands.

mod branch;
mod clone;
mod commit;
pub mod diff;
pub mod gh_cli;
pub mod graphql;
pub mod hooks;
mod issue;
pub mod issue_cache;
pub mod issue_linkage;
mod issue_spec;
pub mod local_spec;
pub mod pr_status;
mod pullrequest;
mod remote;
mod repository;
pub mod stash;
mod submodule;

pub use branch::{Branch, DivergenceStatus};
pub use clone::{extract_repo_name, CloneConfig};
pub use commit::{
    BranchMeta, BranchSummary, ChangeStats, CommitEntry, LoadingState, SectionErrors,
};
pub use diff::{
    detect_base_branch, get_branch_commits, get_branch_diff_files, get_file_diff,
    get_git_change_summary, get_working_tree_status, list_base_branch_candidates, FileChange,
    FileChangeKind, FileDiff, GitChangeSummary, GitViewCommit, WorkingTreeEntry,
};
pub use gh_cli::{create_remote_branch, resolve_remote_branch_sha, PrStatus};
pub use issue::{
    create_linked_branch, create_or_verify_linked_branch, fetch_all_issues_via_rest,
    fetch_issue_detail, fetch_issues_with_options, fetch_open_issues, filter_issues_by_title,
    find_branch_for_issue, find_branches_for_issues, generate_branch_name, is_gh_cli_authenticated,
    is_gh_cli_available, parse_gh_issues_json, resolve_repo_slug, search_issues_with_query,
    FetchIssuesResult, GitHubAssignee, GitHubIssue, GitHubLabel, GitHubMilestone,
    IssueLinkedBranchStatus,
};
pub use issue_spec::{
    append_contract_comment, close_spec_issue, create_spec_issue,
    delete_spec_issue_artifact_comment, get_spec_issue_detail, list_spec_issue_artifact_comments,
    sync_issue_to_project, update_spec_issue, upsert_spec_issue,
    upsert_spec_issue_artifact_comment, ProjectSyncResult, SpecIssueArtifactComment,
    SpecIssueArtifactKind, SpecIssueChecklist, SpecIssueDetail, SpecIssueSections,
    SpecProjectPhase,
};
pub use local_spec::{
    close_local_spec, create_local_spec, delete_local_spec_artifact, get_local_spec_detail,
    list_local_spec_artifacts, list_local_specs, search_local_specs, update_local_spec,
    update_local_spec_phase, upsert_local_spec, upsert_local_spec_artifact, LocalSpecArtifact,
    LocalSpecDetail, LocalSpecMetadata, LocalSpecPhase,
};
pub use pullrequest::{
    PrCache, PrListItem, PrStatusCache, PrStatusInfo, PullRequest, ReviewComment, ReviewInfo,
    WorkflowRunInfo,
};
pub use remote::Remote;
pub use repository::{
    detect_repo_type, get_header_context, get_main_repo_root, is_empty_dir, is_git_repo,
    is_inside_worktree, HeaderContext, RepoType, Repository, WorktreeInfo,
};
pub use stash::{get_stash_list, StashEntry};
pub use submodule::{has_submodules, init_submodules, list_submodules};
