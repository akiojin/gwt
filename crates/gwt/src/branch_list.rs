use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::Path,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchCleanupAvailability {
    Safe,
    Risky,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchCleanupBlockedReason {
    ProtectedBranch,
    CurrentHead,
    ActiveSession,
    RemoteTrackingWithoutLocal,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchCleanupRisk {
    Unmerged,
    RemoteTracking,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchCleanupInfo {
    pub availability: BranchCleanupAvailability,
    pub execution_branch: Option<String>,
    pub merge_target: Option<gwt_git::MergeTarget>,
    pub upstream: Option<String>,
    pub blocked_reason: Option<BranchCleanupBlockedReason>,
    pub risks: Vec<BranchCleanupRisk>,
}

impl Default for BranchCleanupInfo {
    fn default() -> Self {
        Self {
            availability: BranchCleanupAvailability::Blocked,
            execution_branch: None,
            merge_target: None,
            upstream: None,
            blocked_reason: Some(BranchCleanupBlockedReason::Unknown),
            risks: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchScope {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchListEntry {
    pub name: String,
    pub scope: BranchScope,
    pub is_head: bool,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub last_commit_date: Option<String>,
    pub cleanup: BranchCleanupInfo,
}

pub fn list_branch_entries(repo_path: &Path) -> std::io::Result<Vec<BranchListEntry>> {
    list_branch_entries_with_active_sessions(repo_path, &HashSet::new())
}

pub fn list_branch_entries_with_active_sessions(
    repo_path: &Path,
    active_session_branches: &HashSet<String>,
) -> std::io::Result<Vec<BranchListEntry>> {
    let branches = gwt_git::branch::list_branches(repo_path)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let gone_branches = gwt_git::list_gone_branches(repo_path)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let cleanup_targets = build_cleanup_targets(repo_path, &branches, &gone_branches)?;
    Ok(adapt_branches(
        branches,
        active_session_branches,
        &cleanup_targets,
    ))
}

fn build_cleanup_targets(
    repo_path: &Path,
    branches: &[gwt_git::Branch],
    gone_branches: &HashSet<String>,
) -> std::io::Result<HashMap<String, Option<gwt_git::MergeTarget>>> {
    let cleanup_bases = [
        ("main", gwt_git::MergeTarget::Main),
        ("master", gwt_git::MergeTarget::Main),
        ("develop", gwt_git::MergeTarget::Develop),
    ];
    let mut cleanup_targets = HashMap::new();
    for branch in branches.iter().filter(|branch| branch.is_local) {
        let target = gwt_git::detect_cleanable_target(
            repo_path,
            &branch.name,
            &cleanup_bases,
            gone_branches,
        )
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        cleanup_targets.insert(branch.name.clone(), target);
    }
    Ok(cleanup_targets)
}

fn adapt_branches(
    branches: Vec<gwt_git::Branch>,
    active_session_branches: &HashSet<String>,
    cleanup_targets: &HashMap<String, Option<gwt_git::MergeTarget>>,
) -> Vec<BranchListEntry> {
    let current_head_branch = branches
        .iter()
        .find(|branch| branch.is_local && branch.is_head)
        .map(|branch| branch.name.clone());
    let local_upstreams: HashMap<String, Option<String>> = branches
        .iter()
        .filter(|branch| branch.is_local)
        .map(|branch| (branch.name.clone(), branch.upstream.clone()))
        .collect();
    let mut entries: Vec<BranchListEntry> = branches
        .into_iter()
        .map(|branch| {
            let cleanup = build_cleanup_info(
                &branch,
                &local_upstreams,
                current_head_branch.as_deref(),
                active_session_branches,
                cleanup_targets,
            );
            BranchListEntry {
                name: branch.name,
                scope: if branch.is_remote {
                    BranchScope::Remote
                } else {
                    BranchScope::Local
                },
                is_head: branch.is_head,
                upstream: branch.upstream,
                ahead: branch.ahead,
                behind: branch.behind,
                last_commit_date: branch.last_commit_date,
                cleanup,
            }
        })
        .collect();

    entries.sort_by(compare_branch_entries);
    entries
}

fn build_cleanup_info(
    branch: &gwt_git::Branch,
    local_upstreams: &HashMap<String, Option<String>>,
    current_head_branch: Option<&str>,
    active_session_branches: &HashSet<String>,
    cleanup_targets: &HashMap<String, Option<gwt_git::MergeTarget>>,
) -> BranchCleanupInfo {
    let execution_branch = cleanup_execution_branch(branch, local_upstreams);
    let Some(execution_branch_name) = execution_branch.as_deref() else {
        return BranchCleanupInfo {
            availability: BranchCleanupAvailability::Blocked,
            execution_branch: None,
            merge_target: None,
            upstream: None,
            blocked_reason: Some(BranchCleanupBlockedReason::RemoteTrackingWithoutLocal),
            risks: Vec::new(),
        };
    };
    let upstream = local_upstreams
        .get(execution_branch_name)
        .cloned()
        .flatten();

    if gwt_git::is_protected_branch(execution_branch_name) {
        return blocked_cleanup_info(
            execution_branch,
            upstream,
            BranchCleanupBlockedReason::ProtectedBranch,
        );
    }
    if current_head_branch.is_some_and(|head| head == execution_branch_name) {
        return blocked_cleanup_info(
            execution_branch,
            upstream,
            BranchCleanupBlockedReason::CurrentHead,
        );
    }
    if active_session_branches.contains(execution_branch_name) {
        return blocked_cleanup_info(
            execution_branch,
            upstream,
            BranchCleanupBlockedReason::ActiveSession,
        );
    }

    let merge_target = cleanup_targets
        .get(execution_branch_name)
        .cloned()
        .flatten();
    let mut risks = Vec::new();
    if branch.is_remote {
        risks.push(BranchCleanupRisk::RemoteTracking);
    }
    if merge_target.is_none() {
        risks.push(BranchCleanupRisk::Unmerged);
    }

    BranchCleanupInfo {
        availability: if risks.is_empty() {
            BranchCleanupAvailability::Safe
        } else {
            BranchCleanupAvailability::Risky
        },
        execution_branch,
        merge_target,
        upstream,
        blocked_reason: None,
        risks,
    }
}

fn blocked_cleanup_info(
    execution_branch: Option<String>,
    upstream: Option<String>,
    blocked_reason: BranchCleanupBlockedReason,
) -> BranchCleanupInfo {
    BranchCleanupInfo {
        availability: BranchCleanupAvailability::Blocked,
        execution_branch,
        merge_target: None,
        upstream,
        blocked_reason: Some(blocked_reason),
        risks: Vec::new(),
    }
}

fn cleanup_execution_branch(
    branch: &gwt_git::Branch,
    local_upstreams: &HashMap<String, Option<String>>,
) -> Option<String> {
    if branch.is_local {
        return Some(branch.name.clone());
    }
    let local_name = local_branch_for_remote_ref(&branch.name)?;
    let upstream = local_upstreams.get(local_name)?;
    if upstream.as_deref() == Some(branch.name.as_str()) {
        Some(local_name.to_string())
    } else {
        None
    }
}

fn local_branch_for_remote_ref(name: &str) -> Option<&str> {
    name.split_once('/').map(|(_, branch_name)| branch_name)
}

fn compare_branch_entries(left: &BranchListEntry, right: &BranchListEntry) -> Ordering {
    right
        .is_head
        .cmp(&left.is_head)
        .then_with(|| match (left.scope, right.scope) {
            (BranchScope::Local, BranchScope::Remote) => Ordering::Less,
            (BranchScope::Remote, BranchScope::Local) => Ordering::Greater,
            _ => Ordering::Equal,
        })
        .then_with(|| {
            left.name
                .to_ascii_lowercase()
                .cmp(&right.name.to_ascii_lowercase())
        })
        .then_with(|| left.name.cmp(&right.name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapt_branches_sorts_head_then_local_then_remote() {
        let branches = vec![
            gwt_git::Branch {
                name: "origin/main".to_string(),
                is_local: false,
                is_remote: true,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
            },
            gwt_git::Branch {
                name: "feature/zeta".to_string(),
                is_local: true,
                is_remote: false,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
            },
            gwt_git::Branch {
                name: "main".to_string(),
                is_local: true,
                is_remote: false,
                is_head: true,
                upstream: Some("origin/main".to_string()),
                ahead: 0,
                behind: 0,
                last_commit_date: None,
            },
            gwt_git::Branch {
                name: "feature/alpha".to_string(),
                is_local: true,
                is_remote: false,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: None,
            },
        ];

        let entries = adapt_branches(branches, &HashSet::new(), &HashMap::new());
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["main", "feature/alpha", "feature/zeta", "origin/main"]
        );
        assert_eq!(entries[0].scope, BranchScope::Local);
        assert!(entries[0].is_head);
        assert_eq!(entries[3].scope, BranchScope::Remote);
    }
}
