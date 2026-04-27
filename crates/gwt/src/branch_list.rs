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
        let target = gwt_git::detect_cleanable_target(
            repo_path,
            &branch.name,
            branch.upstream.as_deref(),
            gone_branches,
        )
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        cleanup_targets.insert(branch.name.clone(), target);
    }
    Ok(cleanup_targets)
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
    let local_divergence: HashMap<String, (u32, u32)> = entries
        .iter()
        .filter(|branch| branch.scope == BranchScope::Local)
        .map(|branch| (branch.name.clone(), (branch.ahead, branch.behind)))
        .collect();
    let mut entries: Vec<BranchListEntry> = entries
        .into_iter()
        .map(|mut branch| {
            branch.cleanup = build_cleanup_info(
                &branch,
                &local_upstreams,
                &local_divergence,
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
    local_divergence: &HashMap<String, (u32, u32)>,
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
    let execution_divergence = local_divergence
        .get(execution_branch_name)
        .copied()
        .unwrap_or((0, 0));
    if branch.scope == BranchScope::Remote
        && (merge_target.is_none() || execution_divergence.0 > 0 || execution_divergence.1 > 0)
    {
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

fn base_branch_sort_rank(entry: &BranchListEntry) -> Option<(u8, u8)> {
    let branch_name = match entry.scope {
        BranchScope::Local => entry.name.as_str(),
        BranchScope::Remote => local_branch_for_remote_ref(&entry.name)?,
    };
    let base_rank = match branch_name {
        "main" => 0,
        "master" => 1,
        "develop" => 2,
        _ => return None,
    };
    let scope_rank = match entry.scope {
        BranchScope::Local => 0,
        BranchScope::Remote => 1,
    };
    Some((base_rank, scope_rank))
}

fn compare_branch_entries(left: &BranchListEntry, right: &BranchListEntry) -> Ordering {
    match (base_branch_sort_rank(left), base_branch_sort_rank(right)) {
        (Some(left_rank), Some(right_rank)) => {
            return left_rank
                .cmp(&right_rank)
                .then_with(|| compare_branch_names(&left.name, &right.name));
        }
        (Some(_), None) => return Ordering::Less,
        (None, Some(_)) => return Ordering::Greater,
        (None, None) => {}
    }

    compare_branch_commit_dates(&left.last_commit_date, &right.last_commit_date)
        .then_with(|| right.is_head.cmp(&left.is_head))
        .then_with(|| match (left.scope, right.scope) {
            (BranchScope::Local, BranchScope::Remote) => Ordering::Less,
            (BranchScope::Remote, BranchScope::Local) => Ordering::Greater,
            _ => Ordering::Equal,
        })
        .then_with(|| compare_branch_names(&left.name, &right.name))
}

fn compare_branch_names(left: &str, right: &str) -> Ordering {
    left.to_ascii_lowercase()
        .cmp(&right.to_ascii_lowercase())
        .then_with(|| left.cmp(right))
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

    fn make_branch(
        name: &str,
        is_local: bool,
        is_head: bool,
        last_commit_date: Option<&str>,
    ) -> gwt_git::Branch {
        gwt_git::Branch {
            name: name.to_string(),
            is_local,
            is_remote: !is_local,
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: last_commit_date.map(ToString::to_string),
        }
    }

    #[test]
    fn adapt_branches_sorts_base_first_then_newest_head_local() {
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
            vec!["main", "origin/main", "feature/zeta", "feature/alpha"]
        );
        assert_eq!(entries[0].scope, BranchScope::Local);
        assert!(entries[0].is_head);
        assert_eq!(entries[1].scope, BranchScope::Remote);
    }

    #[test]
    fn base_branches_pin_to_top_for_local_and_remote_refs() {
        let branches = vec![
            make_branch(
                "feature/current",
                true,
                true,
                Some("2026-04-21 10:00:00 +0000"),
            ),
            make_branch(
                "origin/develop",
                false,
                false,
                Some("2026-04-02 09:00:00 +0000"),
            ),
            make_branch("develop", true, false, Some("2026-04-01 09:00:00 +0000")),
            make_branch(
                "upstream/main",
                false,
                false,
                Some("2026-04-18 09:00:00 +0000"),
            ),
            make_branch(
                "origin/main",
                false,
                false,
                Some("2026-04-20 09:00:00 +0000"),
            ),
            make_branch(
                "origin/master",
                false,
                false,
                Some("2026-04-07 09:00:00 +0000"),
            ),
            make_branch("master", true, false, Some("2026-04-05 09:00:00 +0000")),
            make_branch("main", true, false, Some("2026-04-15 09:00:00 +0000")),
            make_branch(
                "feature/legacy",
                true,
                false,
                Some("2026-03-01 09:00:00 +0000"),
            ),
        ];

        let entries = adapt_branch_inventory(branches);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(
            &names[..7],
            &[
                "main",
                "origin/main",
                "upstream/main",
                "master",
                "origin/master",
                "develop",
                "origin/develop",
            ]
        );
        assert!(
            names.iter().position(|name| *name == "feature/current")
                > names.iter().position(|name| *name == "origin/develop"),
            "HEAD on a non-base branch must not override base branch pinning"
        );
    }

    #[test]
    fn base_branch_pin_handles_partial_base_set() {
        let branches = vec![
            make_branch("feature/x", true, false, Some("2026-04-20 08:30:00 +0000")),
            make_branch(
                "origin/develop",
                false,
                false,
                Some("2026-04-01 08:30:00 +0000"),
            ),
            make_branch("feature/y", true, true, Some("2026-04-21 08:30:00 +0000")),
        ];

        let entries = adapt_branch_inventory(branches);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(names, vec!["origin/develop", "feature/y", "feature/x"]);
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
