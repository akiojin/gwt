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

pub fn cleanup_selected_branches(
    repo_path: &Path,
    entries: &[BranchListEntry],
    selected_branches: &[String],
    delete_remote: bool,
) -> std::io::Result<Vec<BranchCleanupResultEntry>> {
    let manager = gwt_git::WorktreeManager::new(repo_path);
    let lookup: HashMap<&str, &BranchListEntry> = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry))
        .collect();

    Ok(selected_branches
        .iter()
        .map(|branch_name| {
            let Some(entry) = lookup.get(branch_name.as_str()).copied() else {
                return BranchCleanupResultEntry {
                    branch: branch_name.clone(),
                    execution_branch: None,
                    status: BranchCleanupResultStatus::Failed,
                    message: "Branch not found".to_string(),
                };
            };
            let execution_branch = entry.cleanup.execution_branch.clone();
            let Some(target_branch) = execution_branch.clone() else {
                return BranchCleanupResultEntry {
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
            };
            if entry.cleanup.availability == BranchCleanupAvailability::Blocked {
                return BranchCleanupResultEntry {
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
            }

            match manager.cleanup_branch(&target_branch) {
                Ok(()) => {
                    if delete_remote && entry.cleanup.upstream.is_some() {
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
            }
        })
        .collect())
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
        BranchCleanupBlockedReason::Unknown => "Cannot clean up this branch".to_string(),
    }
}
