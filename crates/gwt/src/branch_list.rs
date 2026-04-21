use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::Path,
};

use chrono::{DateTime, FixedOffset};
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
    pub merge_target: Option<gwt_git::MergeTargetRef>,
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
    #[serde(default)]
    pub cleanup_ready: bool,
    pub cleanup: BranchCleanupInfo,
}

pub fn list_branch_entries(repo_path: &Path) -> std::io::Result<Vec<BranchListEntry>> {
    list_branch_entries_with_active_sessions(repo_path, &HashSet::new())
}

pub fn list_branch_inventory(repo_path: &Path) -> std::io::Result<Vec<BranchListEntry>> {
    let branches = gwt_git::branch::list_branches(repo_path)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(adapt_branch_inventory(branches))
}

pub fn hydrate_branch_entries_with_active_sessions(
    repo_path: &Path,
    entries: Vec<BranchListEntry>,
    active_session_branches: &HashSet<String>,
) -> std::io::Result<Vec<BranchListEntry>> {
    let gone_branches = gwt_git::list_gone_branches(repo_path)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let cleanup_targets = build_cleanup_targets(repo_path, &entries, &gone_branches)?;
    Ok(hydrate_branch_entries(
        entries,
        active_session_branches,
        &cleanup_targets,
    ))
}

pub fn list_branch_entries_with_active_sessions(
    repo_path: &Path,
    active_session_branches: &HashSet<String>,
) -> std::io::Result<Vec<BranchListEntry>> {
    let entries = list_branch_inventory(repo_path)?;
    hydrate_branch_entries_with_active_sessions(repo_path, entries, active_session_branches)
}

fn build_cleanup_targets(
    repo_path: &Path,
    entries: &[BranchListEntry],
    gone_branches: &HashSet<String>,
) -> std::io::Result<HashMap<String, Option<gwt_git::MergeTargetRef>>> {
    let mut cleanup_targets = HashMap::new();
    for branch in entries
        .iter()
        .filter(|branch| branch.scope == BranchScope::Local)
    {
        let cleanup_bases = cleanup_base_candidates(branch);
        let cleanup_bases: Vec<(&str, gwt_git::MergeTarget)> = cleanup_bases
            .iter()
            .map(|(reference, target)| (reference.as_str(), *target))
            .collect();
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

fn cleanup_base_candidates(branch: &BranchListEntry) -> Vec<(String, gwt_git::MergeTarget)> {
    let Some(upstream) = branch.upstream.as_deref() else {
        return Vec::new();
    };
    let Some((remote, _)) = upstream.split_once('/') else {
        return Vec::new();
    };

    let mut bases = Vec::new();
    push_cleanup_base(
        &mut bases,
        format!("{remote}/develop"),
        gwt_git::MergeTarget::Develop,
    );
    push_cleanup_base(
        &mut bases,
        format!("{remote}/main"),
        gwt_git::MergeTarget::Main,
    );
    push_cleanup_base(
        &mut bases,
        format!("{remote}/master"),
        gwt_git::MergeTarget::Main,
    );

    if remote != "origin" {
        push_cleanup_base(
            &mut bases,
            "origin/develop".to_string(),
            gwt_git::MergeTarget::Develop,
        );
        push_cleanup_base(
            &mut bases,
            "origin/main".to_string(),
            gwt_git::MergeTarget::Main,
        );
        push_cleanup_base(
            &mut bases,
            "origin/master".to_string(),
            gwt_git::MergeTarget::Main,
        );
    }

    bases
}

fn push_cleanup_base(
    bases: &mut Vec<(String, gwt_git::MergeTarget)>,
    reference: String,
    target: gwt_git::MergeTarget,
) {
    if bases.iter().any(|(existing_reference, existing_target)| {
        existing_reference == &reference && *existing_target == target
    }) {
        return;
    }
    bases.push((reference, target));
}

fn adapt_branch_inventory(branches: Vec<gwt_git::Branch>) -> Vec<BranchListEntry> {
    let mut entries: Vec<BranchListEntry> = branches
        .into_iter()
        .map(|branch| BranchListEntry {
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
            cleanup_ready: false,
            cleanup: BranchCleanupInfo::default(),
        })
        .collect();

    entries.sort_by(compare_branch_entries);
    entries
}

fn hydrate_branch_entries(
    entries: Vec<BranchListEntry>,
    active_session_branches: &HashSet<String>,
    cleanup_targets: &HashMap<String, Option<gwt_git::MergeTargetRef>>,
) -> Vec<BranchListEntry> {
    let current_head_branch = entries
        .iter()
        .find(|branch| branch.scope == BranchScope::Local && branch.is_head)
        .map(|branch| branch.name.clone());
    let local_upstreams: HashMap<String, Option<String>> = entries
        .iter()
        .filter(|branch| branch.scope == BranchScope::Local)
        .map(|branch| (branch.name.clone(), branch.upstream.clone()))
        .collect();
    let mut entries: Vec<BranchListEntry> = entries
        .into_iter()
        .map(|mut branch| {
            branch.cleanup = build_cleanup_info(
                &branch,
                &local_upstreams,
                current_head_branch.as_deref(),
                active_session_branches,
                cleanup_targets,
            );
            branch.cleanup_ready = true;
            branch
        })
        .collect();

    entries.sort_by(compare_branch_entries);
    entries
}

fn build_cleanup_info(
    branch: &BranchListEntry,
    local_upstreams: &HashMap<String, Option<String>>,
    current_head_branch: Option<&str>,
    active_session_branches: &HashSet<String>,
    cleanup_targets: &HashMap<String, Option<gwt_git::MergeTargetRef>>,
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
    if branch.scope == BranchScope::Remote {
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
    branch: &BranchListEntry,
    local_upstreams: &HashMap<String, Option<String>>,
) -> Option<String> {
    if branch.scope == BranchScope::Local {
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
    compare_branch_commit_dates(&left.last_commit_date, &right.last_commit_date)
        .then_with(|| right.is_head.cmp(&left.is_head))
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

fn compare_branch_commit_dates(left: &Option<String>, right: &Option<String>) -> Ordering {
    match (
        left.as_deref().and_then(parse_branch_commit_date),
        right.as_deref().and_then(parse_branch_commit_date),
    ) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => right.cmp(left),
    }
}

fn parse_branch_commit_date(value: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S %z")
        .ok()
        .or_else(|| DateTime::parse_from_rfc3339(value).ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapt_branches_sorts_newest_first_then_head_local_then_remote() {
        let branches = vec![
            gwt_git::Branch {
                name: "origin/main".to_string(),
                is_local: false,
                is_remote: true,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: Some("2026-04-19 12:00:00 +0000".to_string()),
            },
            gwt_git::Branch {
                name: "feature/zeta".to_string(),
                is_local: true,
                is_remote: false,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: Some("2026-04-20 08:30:00 +0000".to_string()),
            },
            gwt_git::Branch {
                name: "main".to_string(),
                is_local: true,
                is_remote: false,
                is_head: true,
                upstream: Some("origin/main".to_string()),
                ahead: 0,
                behind: 0,
                last_commit_date: Some("2026-04-20 08:30:00 +0000".to_string()),
            },
            gwt_git::Branch {
                name: "feature/alpha".to_string(),
                is_local: true,
                is_remote: false,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: Some("2026-04-18 09:00:00 +0000".to_string()),
            },
        ];

        let entries = adapt_branch_inventory(branches);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["main", "feature/zeta", "origin/main", "feature/alpha"]
        );
        assert_eq!(entries[0].scope, BranchScope::Local);
        assert!(entries[0].is_head);
        assert_eq!(entries[2].scope, BranchScope::Remote);
    }

    #[test]
    fn hydrated_entries_mark_cleanup_ready() {
        let entries = vec![BranchListEntry {
            name: "feature/demo".to_string(),
            scope: BranchScope::Local,
            is_head: false,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: Some("2026-04-20 08:30:00 +0000".to_string()),
            cleanup_ready: false,
            cleanup: BranchCleanupInfo::default(),
        }];
        let cleanup_targets = HashMap::from([(
            String::from("feature/demo"),
            Some(gwt_git::MergeTargetRef::new(
                gwt_git::MergeTarget::Develop,
                "origin/develop",
            )),
        )]);

        let hydrated = hydrate_branch_entries(entries, &HashSet::new(), &cleanup_targets);

        assert!(hydrated[0].cleanup_ready);
        assert_eq!(
            hydrated[0].cleanup.availability,
            BranchCleanupAvailability::Safe
        );
        assert_eq!(
            hydrated[0]
                .cleanup
                .merge_target
                .as_ref()
                .map(|target| target.reference.as_str()),
            Some("origin/develop")
        );
    }
}
