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

            let worktree_path = if is_gwt_workspace_branch(&target_branch) {
                match manager.list() {
                    Ok(worktrees) => worktrees
                        .into_iter()
                        .find(|worktree| worktree.branch.as_deref() == Some(&target_branch))
                        .map(|worktree| worktree.path),
                    Err(error) => {
                        let result = BranchCleanupResultEntry {
                            branch: entry.name.clone(),
                            execution_branch: Some(target_branch),
                            status: BranchCleanupResultStatus::Failed,
                            message: format!("Cleanup failed: {error}"),
                        };
                        emit_result_progress(&mut progress, &result, index, total);
                        return result;
                    }
                }
            } else {
                None
            };

            let cleanup = || {
                if let Some(worktree_path) = worktree_path.as_deref() {
                    manager.cleanup_branch_at_path_with_force_filesystem_delete(
                        &target_branch,
                        worktree_path,
                        options.force_filesystem_delete,
                    )
                } else if options.force_filesystem_delete {
                    manager.cleanup_branch_with_force_filesystem_delete(&target_branch, true)
                } else {
                    manager.cleanup_branch(&target_branch)
                }
                .map_err(|error| std::io::Error::other(error.to_string()))
            };
            let cleanup_result = match worktree_path.as_deref() {
                Some(worktree_path) => {
                    crate::managed_assets::cleanup_worktree_with_codex_project_trust(
                        worktree_path,
                        cleanup,
                    )
                }
                None => cleanup(),
            };
            let result = match cleanup_result {
                Err(error) => {
                    let result = BranchCleanupResultEntry {
                        branch: entry.name.clone(),
                        execution_branch: Some(target_branch),
                        status: BranchCleanupResultStatus::Failed,
                        message: format!("Cleanup failed: {error}"),
                    };
                    emit_result_progress(&mut progress, &result, index, total);
                    return result;
                }
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
    use std::fs;

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
            start_work_eligibility: None,
        }
    }

    fn init_cleanup_repo(repo: &Path) {
        fs::create_dir_all(repo).expect("create repository");
        for args in [
            ["init", "-q"].as_slice(),
            ["config", "user.email", "test@example.com"].as_slice(),
            ["config", "user.name", "Test User"].as_slice(),
            ["commit", "--allow-empty", "-m", "init"].as_slice(),
        ] {
            let output = gwt_core::process::hidden_command("git")
                .args(args)
                .current_dir(repo)
                .output()
                .expect("run git");
            assert!(
                output.status.success(),
                "git {args:?} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    fn project_trust_level(config_path: &Path, project_key: &str) -> Option<String> {
        let root = fs::read_to_string(config_path)
            .ok()
            .and_then(|content| toml::from_str::<toml::Value>(&content).ok())?;
        root.get("projects")?
            .as_table()?
            .get(project_key)?
            .as_table()?
            .get("trust_level")?
            .as_str()
            .map(str::to_string)
    }

    fn local_branch_exists(repo: &Path, branch: &str) -> bool {
        gwt_core::process::hidden_command("git")
            .args([
                "show-ref",
                "--verify",
                "--quiet",
                &format!("refs/heads/{branch}"),
            ])
            .current_dir(repo)
            .status()
            .expect("inspect local branch")
            .success()
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

    #[test]
    fn cleanup_selected_branch_revokes_codex_project_trust_before_removal() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
        let _codex_home =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
        let repo = temp.path().join("repo");
        init_cleanup_repo(&repo);
        let branch = "work/issue-3729";
        let worktree = temp.path().join("issue-3729");
        gwt_git::WorktreeManager::new(&repo)
            .create_from_base("HEAD", branch, &worktree)
            .expect("create managed worktree");
        let config_path = temp.path().join(".codex/config.toml");
        let report = gwt_skills::register_codex_managed_project_trust(&worktree, &config_path)
            .expect("seed Codex project trust");
        let project_key = report.project_path.to_string_lossy().into_owned();
        let mut entry = sample_entry(branch);
        entry.cleanup.availability = BranchCleanupAvailability::Safe;
        entry.cleanup.execution_branch = Some(branch.to_string());

        let results = cleanup_selected_branches(&repo, &[entry], &[branch.to_string()], false);

        assert_eq!(results[0].status, BranchCleanupResultStatus::Success);
        assert!(!worktree.exists(), "eligible managed worktree is removed");
        assert_eq!(
            project_trust_level(&config_path, &project_key),
            None,
            "worktree cleanup must revoke the exact Codex project trust entry"
        );
    }

    #[test]
    fn cleanup_selected_branch_keeps_worktree_when_codex_config_is_malformed() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
        let _codex_home =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
        let repo = temp.path().join("repo");
        init_cleanup_repo(&repo);
        let branch = "work/issue-3730";
        let worktree = temp.path().join("issue-3730");
        gwt_git::WorktreeManager::new(&repo)
            .create_from_base("HEAD", branch, &worktree)
            .expect("create managed worktree");
        let config_path = temp.path().join(".codex/config.toml");
        fs::create_dir_all(config_path.parent().expect("Codex config parent"))
            .expect("create Codex config parent");
        fs::write(&config_path, "projects = [\n").expect("write malformed Codex config");
        let mut entry = sample_entry(branch);
        entry.cleanup.availability = BranchCleanupAvailability::Safe;
        entry.cleanup.execution_branch = Some(branch.to_string());

        let results = cleanup_selected_branches(&repo, &[entry], &[branch.to_string()], false);

        assert_eq!(results[0].status, BranchCleanupResultStatus::Failed);
        assert!(
            results[0].message.contains("Codex project trust"),
            "failure must identify the cleanup boundary: {}",
            results[0].message
        );
        assert!(
            worktree.exists(),
            "trust revocation failure must prevent filesystem deletion"
        );
    }

    #[test]
    fn cleanup_selected_branch_revokes_trust_for_missing_prunable_worktree() {
        let _env_lock = crate::env_test_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let temp = tempdir().expect("tempdir");
        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", temp.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", temp.path());
        let _codex_home =
            gwt_core::test_support::ScopedEnvVar::set("CODEX_HOME", temp.path().join(".codex"));
        let repo = temp.path().join("repo");
        init_cleanup_repo(&repo);
        let branch = "work/issue-3731";
        let worktree = temp.path().join("issue-3731");
        let manager = gwt_git::WorktreeManager::new(&repo);
        manager
            .create_from_base("HEAD", branch, &worktree)
            .expect("create managed worktree");
        let config_path = temp.path().join(".codex/config.toml");
        let project_key = gwt_skills::register_codex_managed_project_trust(&worktree, &config_path)
            .expect("seed Codex project trust")
            .project_path
            .to_string_lossy()
            .into_owned();
        fs::remove_dir_all(&worktree).expect("simulate externally missing worktree path");
        let inventory = manager.list().expect("list stale worktree inventory");
        assert!(inventory.iter().any(|item| {
            item.branch.as_deref() == Some(branch) && item.prunable && !item.path.exists()
        }));
        let mut entry = sample_entry(branch);
        entry.cleanup.availability = BranchCleanupAvailability::Safe;
        entry.cleanup.execution_branch = Some(branch.to_string());

        let results = cleanup_selected_branches(&repo, &[entry], &[branch.to_string()], false);

        assert_eq!(results[0].status, BranchCleanupResultStatus::Success);
        assert_eq!(project_trust_level(&config_path, &project_key), None);
        assert!(
            !local_branch_exists(&repo, branch),
            "existing prune-and-branch-delete recovery must remain usable"
        );
    }
}
