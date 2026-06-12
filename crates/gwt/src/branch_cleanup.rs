use std::{collections::HashMap, path::Path};

use serde::{Deserialize, Serialize};

use crate::{BranchCleanupAvailability, BranchCleanupBlockedReason, BranchListEntry};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchCleanupResultStatus {
    Success,
    Partial,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchCleanupResultEntry {
    pub branch: String,
    pub execution_branch: Option<String>,
    pub status: BranchCleanupResultStatus,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BranchCleanupOptions {
    pub delete_remote: bool,
    pub force_filesystem_delete: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchCleanupProgressPhase {
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchCleanupProgressEntry {
    pub branch: String,
    pub execution_branch: Option<String>,
    pub index: usize,
    pub total: usize,
    pub phase: BranchCleanupProgressPhase,
    pub message: String,
}

pub fn cleanup_selected_branches(
    repo_path: &Path,
    entries: &[BranchListEntry],
    selected_branches: &[String],
    delete_remote: bool,
) -> Vec<BranchCleanupResultEntry> {
    cleanup_selected_branches_with_options(
        repo_path,
        entries,
        selected_branches,
        BranchCleanupOptions {
            delete_remote,
            force_filesystem_delete: false,
        },
    )
}

pub fn cleanup_selected_branches_with_options(
    repo_path: &Path,
    entries: &[BranchListEntry],
    selected_branches: &[String],
    options: BranchCleanupOptions,
) -> Vec<BranchCleanupResultEntry> {
    cleanup_selected_branches_with_progress(repo_path, entries, selected_branches, options, |_| {})
}

pub fn cleanup_selected_branches_with_progress(
    repo_path: &Path,
    entries: &[BranchListEntry],
    selected_branches: &[String],
    options: BranchCleanupOptions,
    mut progress: impl FnMut(BranchCleanupProgressEntry),
) -> Vec<BranchCleanupResultEntry> {
    let git_root = git_command_root(repo_path);
    let manager = gwt_git::WorktreeManager::new(&git_root);
    let lookup: HashMap<&str, &BranchListEntry> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();
    let total = selected_branches.len();

    selected_branches
        .iter()
        .enumerate()
        .map(|(offset, branch_name)| {
            let index = offset + 1;
            let Some(entry) = lookup.get(branch_name.as_str()).copied() else {
                let result = BranchCleanupResultEntry {
                    branch: branch_name.clone(),
                    execution_branch: None,
                    status: BranchCleanupResultStatus::Failed,
                    message: "Branch not found".to_string(),
                };
                emit_result_progress(&mut progress, &result, index, total);
                return result;
            };
            let execution_branch = entry.cleanup.execution_branch.clone();
            progress(BranchCleanupProgressEntry {
                branch: entry.name.clone(),
                execution_branch: execution_branch.clone(),
                index,
                total,
                phase: BranchCleanupProgressPhase::Running,
                message: format!("Cleaning {}", entry.name),
            });
            let Some(target_branch) = execution_branch.clone() else {
                let result = BranchCleanupResultEntry {
                    branch: entry.name.clone(),
                    execution_branch,
                    status: BranchCleanupResultStatus::Failed,
                    message: blocked_reason_message(
                        entry
                            .cleanup
                            .blocked_reason
                            .unwrap_or(BranchCleanupBlockedReason::Unknown),
                    ),
                };
                emit_result_progress(&mut progress, &result, index, total);
                return result;
            };
            if entry.cleanup.availability == BranchCleanupAvailability::Blocked {
                let result = BranchCleanupResultEntry {
                    branch: entry.name.clone(),
                    execution_branch: Some(target_branch),
                    status: BranchCleanupResultStatus::Failed,
                    message: blocked_reason_message(
                        entry
                            .cleanup
                            .blocked_reason
                            .unwrap_or(BranchCleanupBlockedReason::Unknown),
                    ),
                };
                emit_result_progress(&mut progress, &result, index, total);
                return result;
            }
            // SPEC-2009 FR-070: protected base branches (main/master/develop)
            // are not `work/*` but are allowed for LOCAL cleanup. Everything
            // else still must be a gwt-managed workspace branch.
            if !is_gwt_workspace_branch(&target_branch)
                && !gwt_git::is_protected_branch(&target_branch)
            {
                let result = BranchCleanupResultEntry {
                    branch: entry.name.clone(),
                    execution_branch: Some(target_branch),
                    status: BranchCleanupResultStatus::Failed,
                    message: blocked_reason_message(BranchCleanupBlockedReason::NonWorkspaceBranch),
                };
                emit_result_progress(&mut progress, &result, index, total);
                return result;
            }

            let cleanup_result = if options.force_filesystem_delete {
                manager.cleanup_branch_with_force_filesystem_delete(&target_branch, true)
            } else {
                manager.cleanup_branch(&target_branch)
            };
            let result = match cleanup_result {
                Ok(()) => {
                    // SPEC-2009 FR-071: never delete a protected base branch
                    // from the remote, regardless of the delete-remote flag.
                    if options.delete_remote
                        && entry.cleanup.upstream.is_some()
                        && !gwt_git::is_protected_branch(&target_branch)
                    {
                        match manager
                            .delete_remote_branch(&target_branch, entry.cleanup.upstream.as_deref())
                        {
                            Ok(gwt_git::RemoteDeleteOutcome::Deleted) => BranchCleanupResultEntry {
                                branch: entry.name.clone(),
                                execution_branch: Some(target_branch),
                                status: BranchCleanupResultStatus::Success,
                                message: "Deleted local and remote branches".to_string(),
                            },
                            Ok(gwt_git::RemoteDeleteOutcome::SkippedMissing) => {
                                BranchCleanupResultEntry {
                                    branch: entry.name.clone(),
                                    execution_branch: Some(target_branch),
                                    status: BranchCleanupResultStatus::Success,
                                    message:
                                        "Deleted local branch; remote branch was already missing"
                                            .to_string(),
                                }
                            }
                            Err(error) => BranchCleanupResultEntry {
                                branch: entry.name.clone(),
                                execution_branch: Some(target_branch),
                                status: BranchCleanupResultStatus::Partial,
                                message: format!(
                                    "Deleted local branch; remote delete failed: {error}"
                                ),
                            },
                        }
                    } else {
                        BranchCleanupResultEntry {
                            branch: entry.name.clone(),
                            execution_branch: Some(target_branch),
                            status: BranchCleanupResultStatus::Success,
                            message: "Deleted local branch".to_string(),
                        }
                    }
                }
                Err(error) => BranchCleanupResultEntry {
                    branch: entry.name.clone(),
                    execution_branch: Some(target_branch),
                    status: BranchCleanupResultStatus::Failed,
                    message: format!("Cleanup failed: {error}"),
                },
            };
            emit_result_progress(&mut progress, &result, index, total);
            result
        })
        .collect()
}

fn emit_result_progress(
    progress: &mut impl FnMut(BranchCleanupProgressEntry),
    result: &BranchCleanupResultEntry,
    index: usize,
    total: usize,
) {
    let phase = match result.status {
        BranchCleanupResultStatus::Success | BranchCleanupResultStatus::Partial => {
            BranchCleanupProgressPhase::Done
        }
        BranchCleanupResultStatus::Failed => BranchCleanupProgressPhase::Failed,
    };
    progress(BranchCleanupProgressEntry {
        branch: result.branch.clone(),
        execution_branch: result.execution_branch.clone(),
        index,
        total,
        phase,
        message: result.message.clone(),
    });
}

fn is_gwt_workspace_branch(branch_name: &str) -> bool {
    branch_name.starts_with("work/")
}

fn git_command_root(repo_path: &Path) -> std::path::PathBuf {
    gwt_git::worktree::main_worktree_root(repo_path).unwrap_or_else(|_| repo_path.to_path_buf())
}

fn blocked_reason_message(reason: BranchCleanupBlockedReason) -> String {
    match reason {
        BranchCleanupBlockedReason::ProtectedBranch => {
            "Cannot clean up a protected branch".to_string()
        }
        BranchCleanupBlockedReason::CurrentHead => {
            "Cannot clean up the current HEAD branch".to_string()
        }
        BranchCleanupBlockedReason::ActiveSession => {
            "Cannot clean up a branch with an active session".to_string()
        }
        BranchCleanupBlockedReason::RemoteTrackingWithoutLocal => {
            "Cannot clean up a remote-tracking branch without a local counterpart".to_string()
        }
        BranchCleanupBlockedReason::NonWorkspaceBranch => {
            "Only gwt-managed workspaces can be cleaned up".to_string()
        }
        BranchCleanupBlockedReason::Unknown => "Cannot clean up this branch".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::{BranchCleanupAvailability, BranchCleanupInfo, BranchScope};

    fn sample_entry(name: &str) -> BranchListEntry {
        BranchListEntry {
            name: name.to_string(),
            scope: BranchScope::Local,
            is_head: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: None,
            cleanup_ready: true,
            cleanup: BranchCleanupInfo::default(),
            resume: crate::BranchResumeInfo::unavailable(),
        }
    }

    #[test]
    fn cleanup_selected_branches_reports_missing_branch() {
        let repo = tempdir().expect("tempdir");

        let results =
            cleanup_selected_branches(repo.path(), &[], &[String::from("feature/missing")], false);

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].branch, "feature/missing");
        assert_eq!(results[0].status, BranchCleanupResultStatus::Failed);
        assert_eq!(results[0].message, "Branch not found");
    }

    #[test]
    fn cleanup_selected_branches_uses_blocked_reason_when_execution_branch_is_missing() {
        let repo = tempdir().expect("tempdir");
        let mut entry = sample_entry("feature/demo");
        entry.cleanup.blocked_reason = Some(BranchCleanupBlockedReason::ActiveSession);

        let results = cleanup_selected_branches(
            repo.path(),
            &[entry],
            &[String::from("feature/demo")],
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].execution_branch, None);
        assert_eq!(results[0].status, BranchCleanupResultStatus::Failed);
        assert_eq!(
            results[0].message,
            "Cannot clean up a branch with an active session"
        );
    }

    #[test]
    fn blocked_reason_message_covers_all_variants() {
        assert_eq!(
            blocked_reason_message(BranchCleanupBlockedReason::ProtectedBranch),
            "Cannot clean up a protected branch"
        );
        assert_eq!(
            blocked_reason_message(BranchCleanupBlockedReason::CurrentHead),
            "Cannot clean up the current HEAD branch"
        );
        assert_eq!(
            blocked_reason_message(BranchCleanupBlockedReason::ActiveSession),
            "Cannot clean up a branch with an active session"
        );
        assert_eq!(
            blocked_reason_message(BranchCleanupBlockedReason::RemoteTrackingWithoutLocal),
            "Cannot clean up a remote-tracking branch without a local counterpart"
        );
        assert_eq!(
            blocked_reason_message(BranchCleanupBlockedReason::NonWorkspaceBranch),
            "Only gwt-managed workspaces can be cleaned up"
        );
        assert_eq!(
            blocked_reason_message(BranchCleanupBlockedReason::Unknown),
            "Cannot clean up this branch"
        );
    }

    #[test]
    fn cleanup_selected_branches_preserves_blocked_execution_branch_message() {
        let repo = tempdir().expect("tempdir");
        let mut entry = sample_entry("feature/demo");
        entry.cleanup.availability = BranchCleanupAvailability::Blocked;
        entry.cleanup.execution_branch = Some("feature/demo".to_string());
        entry.cleanup.blocked_reason = Some(BranchCleanupBlockedReason::ProtectedBranch);

        let results = cleanup_selected_branches(
            repo.path(),
            &[entry],
            &[String::from("feature/demo")],
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].execution_branch.as_deref(), Some("feature/demo"));
        assert_eq!(results[0].status, BranchCleanupResultStatus::Failed);
        assert_eq!(results[0].message, "Cannot clean up a protected branch");
    }

    #[test]
    fn cleanup_selected_branches_preserves_non_workspace_branch() {
        let repo = tempdir().expect("tempdir");
        let mut entry = sample_entry("feature/demo");
        entry.cleanup.availability = BranchCleanupAvailability::Safe;
        entry.cleanup.execution_branch = Some("feature/demo".to_string());

        let results = cleanup_selected_branches(
            repo.path(),
            &[entry],
            &[String::from("feature/demo")],
            false,
        );

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].execution_branch.as_deref(), Some("feature/demo"));
        assert_eq!(results[0].status, BranchCleanupResultStatus::Failed);
        assert_eq!(
            results[0].message,
            "Only gwt-managed workspaces can be cleaned up"
        );
    }
}
