use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    path::Path,
};

use chrono::{DateTime, FixedOffset};
use serde::{Deserialize, Serialize};

type RemoteBaseBranchRanks = HashMap<String, u8>;

// SPEC-2009 FR-067: monotonic id for one Branches detail-check load. The
// inventory and the matching hydrated event of a single load share the id; the
// frontend ignores any branch_entries event whose load_id is older than the
// newest it has applied, so an evict/reconnect cannot let a stale in-flight
// load overwrite fresh data. `Ordering` here is `cmp::Ordering` (imported
// above), so the atomic ordering is fully qualified to avoid the name clash.
static BRANCH_LOAD_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Returns the next monotonic Branches detail-check load id (SPEC-2009 FR-067).
pub fn next_branch_load_id() -> u64 {
    BRANCH_LOAD_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

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
    NonWorkspaceBranch,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchCleanupRisk {
    Unmerged,
    RemoteTracking,
    // SPEC-2009 FR-070: a protected base branch (main/master/develop) selectable
    // for LOCAL cleanup only — its remote counterpart is always protected.
    ProtectedBase,
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
pub struct BranchResumeInfo {
    pub available: bool,
    pub reason: Option<String>,
}

impl BranchResumeInfo {
    pub fn available() -> Self {
        Self {
            available: true,
            reason: None,
        }
    }

    pub fn unavailable() -> Self {
        Self {
            available: false,
            reason: Some("No resumable session".to_string()),
        }
    }
}

impl Default for BranchResumeInfo {
    fn default() -> Self {
        Self::unavailable()
    }
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
    #[serde(default)]
    pub resume: BranchResumeInfo,
    /// SPEC-2359 US-83 / FR-443: whether this row is an eligible "Start Work /
    /// Open" source for an existing remote branch. Populated for remote-tracking
    /// rows during hydration; `None` on local rows and on payloads from older
    /// backends (serde default).
    #[serde(default)]
    pub start_work_eligibility: Option<RemoteStartWorkEligibility>,
}

pub fn list_branch_entries(repo_path: &Path) -> std::io::Result<Vec<BranchListEntry>> {
    list_branch_entries_with_active_sessions(repo_path, &HashSet::new())
}

pub fn list_branch_inventory(repo_path: &Path) -> std::io::Result<Vec<BranchListEntry>> {
    let git_root = git_command_root(repo_path)?;
    let branches = gwt_git::branch::list_branches(&git_root)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(adapt_branch_inventory(branches))
}

pub fn hydrate_branch_entries_with_active_sessions(
    repo_path: &Path,
    entries: Vec<BranchListEntry>,
    active_session_branches: &HashSet<String>,
) -> std::io::Result<Vec<BranchListEntry>> {
    let git_root = git_command_root(repo_path)?;
    let gone_branches = gwt_git::list_gone_branches(&git_root)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let cleanup_targets = build_cleanup_targets(&git_root, &entries, &gone_branches)?;
    // SPEC-2009 FR-070: a bare repo's symbolic HEAD (e.g. main) is NOT a real
    // worktree checkout — gwt resolves the default branch from origin/HEAD — so
    // it must not block its branch from local cleanup.
    let head_is_real_checkout = !repo_root_is_bare(&git_root);
    Ok(hydrate_branch_entries(
        entries,
        active_session_branches,
        &cleanup_targets,
        head_is_real_checkout,
    ))
}

fn repo_root_is_bare(git_root: &Path) -> bool {
    gwt_git::Repository::open(git_root)
        .map(|repo| repo.is_bare())
        .unwrap_or(false)
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
    let remote_names = gwt_git::list_remote_names(repo_path).unwrap_or_default();
    let mut cleanup_targets = HashMap::new();
    for branch in entries
        .iter()
        .filter(|branch| branch.scope == BranchScope::Local)
    {
        let target = gwt_git::detect_cleanable_target_with_remote_names(
            repo_path,
            &branch.name,
            branch.upstream.as_deref(),
            gone_branches,
            &remote_names,
        )
        .map_err(|error| std::io::Error::other(error.to_string()))?;
        cleanup_targets.insert(branch.name.clone(), target);
    }
    Ok(cleanup_targets)
}

fn git_command_root(repo_path: &Path) -> std::io::Result<std::path::PathBuf> {
    gwt_git::worktree::main_worktree_root(repo_path)
        .map_err(|error| std::io::Error::other(error.to_string()))
}

fn adapt_branch_inventory(branches: Vec<gwt_git::Branch>) -> Vec<BranchListEntry> {
    let remote_base_branch_ranks = remote_base_branch_ranks(&branches);
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
            resume: BranchResumeInfo::unavailable(),
            start_work_eligibility: None,
        })
        .collect();

    entries.sort_by(|left, right| compare_branch_entries(left, right, &remote_base_branch_ranks));
    entries
}

fn hydrate_branch_entries(
    entries: Vec<BranchListEntry>,
    active_session_branches: &HashSet<String>,
    cleanup_targets: &HashMap<String, Option<gwt_git::MergeTargetRef>>,
    head_is_real_checkout: bool,
) -> Vec<BranchListEntry> {
    // Only a real worktree checkout blocks cleanup as the current HEAD. In a
    // bare repository the symbolic HEAD is not a checkout, so it must not block
    // its branch (SPEC-2009 FR-070).
    let current_head_branch = if head_is_real_checkout {
        entries
            .iter()
            .find(|branch| branch.scope == BranchScope::Local && branch.is_head)
            .map(|branch| branch.name.clone())
    } else {
        None
    };
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
    // SPEC-2359 US-83 / FR-443: classify remote rows for the "Start Work / Open"
    // entry points before consuming the entries. Pure — no git spawn.
    let start_work_eligibility: HashMap<String, RemoteStartWorkEligibility> =
        remote_start_work_eligibility(&entries, active_session_branches)
            .into_iter()
            .collect();
    let entries: Vec<BranchListEntry> = entries
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
            branch.start_work_eligibility = start_work_eligibility.get(&branch.name).copied();
            branch
        })
        .collect();

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

    // CurrentHead / ActiveSession take precedence over the protected-base
    // policy below: a checked-out or in-use protected branch stays Blocked.
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
    // SPEC-2009 FR-070: protected base branches (main/master/develop) are
    // selectable for LOCAL cleanup as Risky — remote deletion stays protected
    // (enforced in branch_cleanup execution and gwt-git delete_remote_branch).
    // They are Risky (not Safe), so "Select all safe" never sweeps them up
    // (FR-072).
    if gwt_git::is_protected_branch(execution_branch_name) {
        let merge_target = cleanup_targets
            .get(execution_branch_name)
            .cloned()
            .flatten();
        return BranchCleanupInfo {
            availability: BranchCleanupAvailability::Risky,
            execution_branch,
            merge_target,
            upstream,
            blocked_reason: None,
            risks: vec![BranchCleanupRisk::ProtectedBase],
        };
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

/// SPEC-2359 US-83 / FR-443: per-remote-row eligibility for "Start Work / Open"
/// from an existing branch. Pure classification over already-assembled branch
/// entries — performs no git spawn, so it is safe to call during view assembly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteStartWorkEligibility {
    /// Offer "Start Work / Open": an origin branch with no local counterpart and
    /// no live session — launching materializes a worktree on the branch itself.
    StartWork,
    /// The branch already exists locally or has a live Work; surface focus /
    /// Resume instead of minting a second Work (FR-332 / US-79).
    ResumeExisting,
    /// Not offered as a Start Work source: a non-origin remote (FR-447), a
    /// protected base branch (main/master/develop), or a non-branch ref such as
    /// `origin/HEAD`.
    Hidden,
}

/// Classify each remote-tracking entry in `entries` for the "Start Work from an
/// existing remote branch" entry points (SPEC-2359 US-83 / FR-443).
///
/// Only `origin` remote branches are eligible. Protected base branches and refs
/// without a branch name are `Hidden`. A branch that already has a local
/// counterpart or a live session is `ResumeExisting`, so the UI surfaces
/// focus/Resume rather than a duplicate Work. The result is keyed by the remote
/// entry name (e.g. `"origin/feature-foo"`).
pub fn remote_start_work_eligibility(
    entries: &[BranchListEntry],
    active_session_branches: &HashSet<String>,
) -> Vec<(String, RemoteStartWorkEligibility)> {
    let local_branch_names: HashSet<&str> = entries
        .iter()
        .filter(|entry| entry.scope == BranchScope::Local)
        .map(|entry| entry.name.as_str())
        .collect();
    entries
        .iter()
        .filter(|entry| entry.scope == BranchScope::Remote)
        .map(|entry| {
            let eligibility = classify_remote_start_work_eligibility(
                &entry.name,
                &local_branch_names,
                active_session_branches,
            );
            (entry.name.clone(), eligibility)
        })
        .collect()
}

/// SPEC-2359 US-83 / FR-444: the most-recently-updated eligible remote branches
/// a user can start work on, used to populate the Workspace "Start work on a
/// branch" rows. Derived from [`remote_start_work_eligibility`], then sorted by
/// most-recent commit first and capped at [`REMOTE_START_WORK_BRANCH_CAP`] — a
/// busy repo can have hundreds of stale remote refs, and flooding the Workspace
/// list with all of them is worse than surfacing the recently-active ones.
pub fn eligible_remote_start_work_branch_names(
    entries: &[BranchListEntry],
    active_session_branches: &HashSet<String>,
) -> Vec<String> {
    let eligible: HashSet<String> = remote_start_work_eligibility(entries, active_session_branches)
        .into_iter()
        .filter(|(_, eligibility)| *eligibility == RemoteStartWorkEligibility::StartWork)
        .map(|(name, _)| name)
        .collect();
    let mut rows: Vec<&BranchListEntry> = entries
        .iter()
        .filter(|entry| eligible.contains(&entry.name))
        // Drop dependency-bot branches (dependabot/* , renovate/*): you review
        // those via their PR, you don't "start fresh work" on the branch, so
        // surfacing them — often the most recently pushed — as the top Start
        // Work suggestions is noise.
        .filter(|entry| !is_bot_branch(&entry.name))
        .collect();
    // `last_commit_date` is a sortable `YYYY-MM-DD HH:MM:SS ...` string; reverse
    // (b vs a) puts the newest first. Missing dates sort last.
    rows.sort_by(|a, b| b.last_commit_date.cmp(&a.last_commit_date));
    rows.into_iter()
        .take(REMOTE_START_WORK_BRANCH_CAP)
        .map(|entry| entry.name.clone())
        .collect()
}

/// Dependency-bot branches are reviewed through their PR, not continued as a
/// fresh worktree, so they are excluded from the Workspace Start Work rows.
fn is_bot_branch(remote_entry_name: &str) -> bool {
    let branch = remote_entry_name
        .split_once('/')
        .map(|(_, branch)| branch)
        .unwrap_or(remote_entry_name);
    branch.starts_with("dependabot/") || branch.starts_with("renovate/")
}

/// SPEC-2359 US-83: cap on the eligible remote branches surfaced in the
/// Workspace "Start work on a branch" rows so a repo with hundreds of stale
/// remote refs does not flood the list.
pub const REMOTE_START_WORK_BRANCH_CAP: usize = 20;

fn classify_remote_start_work_eligibility(
    remote_entry_name: &str,
    local_branch_names: &HashSet<&str>,
    active_session_branches: &HashSet<String>,
) -> RemoteStartWorkEligibility {
    let Some((remote, branch_name)) = remote_entry_name.split_once('/') else {
        return RemoteStartWorkEligibility::Hidden;
    };
    // origin only — fork/upstream remotes are Out of Scope (FR-447).
    if remote != "origin" || branch_name.is_empty() || branch_name == "HEAD" {
        return RemoteStartWorkEligibility::Hidden;
    }
    // Never start work directly on a protected base branch (FR-443).
    if gwt_git::is_protected_branch(remote_entry_name) {
        return RemoteStartWorkEligibility::Hidden;
    }
    // Already materialized locally or running → focus/Resume, not a 2nd Work.
    if local_branch_names.contains(branch_name) || active_session_branches.contains(branch_name) {
        return RemoteStartWorkEligibility::ResumeExisting;
    }
    RemoteStartWorkEligibility::StartWork
}

fn remote_base_branch_ranks(branches: &[gwt_git::Branch]) -> RemoteBaseBranchRanks {
    branches
        .iter()
        .filter(|branch| branch.is_remote)
        .filter_map(|branch| {
            let branch_name = branch
                .remote_branch_name
                .as_deref()
                .or_else(|| local_branch_for_remote_ref(&branch.name))?;
            Some((branch.name.clone(), base_branch_rank(branch_name)?))
        })
        .collect()
}

fn base_branch_rank(branch_name: &str) -> Option<u8> {
    Some(match branch_name {
        "main" => 0,
        "master" => 1,
        "develop" => 2,
        _ => return None,
    })
}

fn base_branch_sort_rank(
    entry: &BranchListEntry,
    remote_base_branch_ranks: &RemoteBaseBranchRanks,
) -> Option<(u8, u8)> {
    let base_rank = match entry.scope {
        BranchScope::Local => base_branch_rank(&entry.name)?,
        BranchScope::Remote => *remote_base_branch_ranks.get(&entry.name)?,
    };
    let scope_rank = match entry.scope {
        BranchScope::Local => 0,
        BranchScope::Remote => 1,
    };
    Some((base_rank, scope_rank))
}

fn compare_branch_entries(
    left: &BranchListEntry,
    right: &BranchListEntry,
    remote_base_branch_ranks: &RemoteBaseBranchRanks,
) -> Ordering {
    match (
        base_branch_sort_rank(left, remote_base_branch_ranks),
        base_branch_sort_rank(right, remote_base_branch_ranks),
    ) {
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
    left.bytes()
        .map(|byte| byte.to_ascii_lowercase())
        .cmp(right.bytes().map(|byte| byte.to_ascii_lowercase()))
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

    #[test]
    fn next_branch_load_id_is_strictly_increasing() {
        // SPEC-2009 FR-067: ids must be monotonic so the frontend can drop a
        // stale (older) load delivered out of order after an evict/reconnect.
        let a = next_branch_load_id();
        let b = next_branch_load_id();
        let c = next_branch_load_id();
        assert!(a < b, "expected {a} < {b}");
        assert!(b < c, "expected {b} < {c}");
    }

    fn make_branch(
        name: &str,
        is_local: bool,
        is_head: bool,
        last_commit_date: Option<&str>,
    ) -> gwt_git::Branch {
        let (remote_name, remote_branch_name) = if is_local {
            (None, None)
        } else {
            name.split_once('/')
                .map_or((None, None), |(remote, branch)| {
                    (Some(remote.to_string()), Some(branch.to_string()))
                })
        };
        gwt_git::Branch {
            name: name.to_string(),
            remote_name,
            remote_branch_name,
            is_local,
            is_remote: !is_local,
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: last_commit_date.map(ToString::to_string),
        }
    }

    fn make_remote_branch(
        name: &str,
        remote_name: &str,
        remote_branch_name: &str,
        last_commit_date: Option<&str>,
    ) -> gwt_git::Branch {
        gwt_git::Branch {
            remote_name: Some(remote_name.to_string()),
            remote_branch_name: Some(remote_branch_name.to_string()),
            ..make_branch(name, false, false, last_commit_date)
        }
    }

    #[test]
    fn eligible_remote_start_work_branch_names_drops_dependency_bot_branches() {
        // SPEC-2359 US-83: dependabot/renovate branches are reviewed via PR, not
        // continued as fresh work, so they must not surface as Start Work rows.
        let entries = adapt_branch_inventory(vec![
            make_branch(
                "origin/dependabot/cargo/serde-1.0",
                false,
                false,
                Some("2026-05-01 00:00:00 +0000"),
            ),
            make_branch(
                "origin/renovate/eslint",
                false,
                false,
                Some("2026-05-02 00:00:00 +0000"),
            ),
            make_branch(
                "origin/feature/real-work",
                false,
                false,
                Some("2026-04-01 00:00:00 +0000"),
            ),
        ]);
        let names = eligible_remote_start_work_branch_names(&entries, &HashSet::new());
        assert_eq!(
            names,
            vec!["origin/feature/real-work".to_string()],
            "bot branches are excluded even though they are the most recent"
        );
    }

    #[test]
    fn eligible_remote_start_work_branch_names_caps_and_sorts_by_recency() {
        // SPEC-2359 US-83: a busy repo can have hundreds of eligible remote
        // refs; the Workspace rows surface only the most recent, capped.
        let branches = (0..25)
            .map(|i| {
                make_branch(
                    &format!("origin/feature/b{i:02}"),
                    false,
                    false,
                    Some(&format!("2026-04-{:02} 00:00:00 +0000", i + 1)),
                )
            })
            .collect();
        let entries = adapt_branch_inventory(branches);
        let names = eligible_remote_start_work_branch_names(&entries, &HashSet::new());

        assert_eq!(
            names.len(),
            REMOTE_START_WORK_BRANCH_CAP,
            "capped at the limit"
        );
        assert_eq!(
            names.first().map(String::as_str),
            Some("origin/feature/b24"),
            "most recent branch is listed first"
        );
        assert!(
            !names.iter().any(|name| name == "origin/feature/b00"),
            "the oldest branches are dropped past the cap"
        );
    }

    #[test]
    fn eligible_remote_start_work_branch_names_lists_only_fresh_origin_branches() {
        // SPEC-2359 US-83 / FR-444: the wizard picker offers only fresh origin
        // branches — protected base, local rows, and non-origin remotes drop out.
        let entries = adapt_branch_inventory(vec![
            make_branch("origin/feature/fresh", false, false, None),
            make_branch("origin/develop", false, false, None),
            make_branch("upstream/feature/fork", false, false, None),
            make_branch("feature/local", true, false, None),
        ]);
        let names = eligible_remote_start_work_branch_names(&entries, &HashSet::new());
        assert_eq!(names, vec!["origin/feature/fresh".to_string()]);
    }

    #[test]
    fn hydrate_attaches_start_work_eligibility_to_remote_rows() {
        // SPEC-2359 US-83 / FR-443: hydration carries per-remote-row eligibility
        // to the Branches payload so the frontend can render "Start Work / Open".
        let entries = adapt_branch_inventory(vec![
            make_branch("origin/feature/fresh", false, false, None),
            make_branch("feature/local", true, false, None),
        ]);
        let hydrated = hydrate_branch_entries(entries, &HashSet::new(), &HashMap::new(), true);

        let fresh = hydrated
            .iter()
            .find(|entry| entry.name == "origin/feature/fresh")
            .expect("remote row present");
        assert_eq!(
            fresh.start_work_eligibility,
            Some(RemoteStartWorkEligibility::StartWork),
            "a fresh origin row carries StartWork eligibility for the frontend"
        );

        let local = hydrated
            .iter()
            .find(|entry| entry.name == "feature/local")
            .expect("local row present");
        assert!(
            local.start_work_eligibility.is_none(),
            "local rows carry no start-work eligibility"
        );
    }

    #[test]
    fn remote_start_work_eligibility_classifies_origin_protected_and_existing() {
        // SPEC-2359 US-83 / FR-443 / SC-302: only fresh origin branches are
        // "Start Work / Open" sources. Protected base and non-origin remotes are
        // Hidden; a branch with a local counterpart or a live session resumes.
        let entries = adapt_branch_inventory(vec![
            make_branch("feature/alpha", true, false, None),
            gwt_git::Branch {
                upstream: Some("origin/feature/alpha".to_string()),
                ..make_branch("origin/feature/alpha", false, false, None)
            },
            make_branch("origin/feature/fresh", false, false, None),
            make_branch("origin/develop", false, false, None),
            make_branch("origin/feature/busy", false, false, None),
            make_branch("upstream/feature/fork", false, false, None),
        ]);
        let mut active = HashSet::new();
        active.insert("feature/busy".to_string());

        let eligibility: HashMap<String, RemoteStartWorkEligibility> =
            remote_start_work_eligibility(&entries, &active)
                .into_iter()
                .collect();

        assert_eq!(
            eligibility.get("origin/feature/fresh"),
            Some(&RemoteStartWorkEligibility::StartWork),
            "a fresh origin branch with no local copy is a Start Work source"
        );
        assert_eq!(
            eligibility.get("origin/feature/alpha"),
            Some(&RemoteStartWorkEligibility::ResumeExisting),
            "an origin branch with a local counterpart resumes instead"
        );
        assert_eq!(
            eligibility.get("origin/feature/busy"),
            Some(&RemoteStartWorkEligibility::ResumeExisting),
            "an origin branch with a live session resumes instead"
        );
        assert_eq!(
            eligibility.get("origin/develop"),
            Some(&RemoteStartWorkEligibility::Hidden),
            "a protected base branch is never a Start Work source"
        );
        assert_eq!(
            eligibility.get("upstream/feature/fork"),
            Some(&RemoteStartWorkEligibility::Hidden),
            "non-origin remotes are out of scope"
        );
        // Local-only rows are never offered as remote Start Work sources.
        assert!(!eligibility.contains_key("feature/alpha"));
    }

    #[test]
    fn adapt_branches_sorts_base_first_then_newest_head_local() {
        let branches = vec![
            make_branch(
                "origin/main",
                false,
                false,
                Some("2026-04-19 12:00:00 +0000"),
            ),
            make_branch(
                "feature/zeta",
                true,
                false,
                Some("2026-04-20 08:30:00 +0000"),
            ),
            gwt_git::Branch {
                upstream: Some("origin/main".to_string()),
                ..make_branch("main", true, true, Some("2026-04-20 08:30:00 +0000"))
            },
            make_branch(
                "feature/alpha",
                true,
                false,
                Some("2026-04-18 09:00:00 +0000"),
            ),
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
    fn base_branch_pin_uses_remote_metadata_for_slash_remotes() {
        let branches = vec![
            make_branch(
                "feature/current",
                true,
                true,
                Some("2026-04-21 10:00:00 +0000"),
            ),
            make_remote_branch(
                "origin/feature/main",
                "origin",
                "feature/main",
                Some("2026-04-20 10:00:00 +0000"),
            ),
            make_remote_branch(
                "team/core/main",
                "team/core",
                "main",
                Some("2026-04-01 10:00:00 +0000"),
            ),
        ];

        let entries = adapt_branch_inventory(branches);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["team/core/main", "feature/current", "origin/feature/main"]
        );
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
            resume: BranchResumeInfo::unavailable(),
            start_work_eligibility: None,
        }];
        let cleanup_targets = HashMap::from([(
            String::from("feature/demo"),
            Some(gwt_git::MergeTargetRef::new(
                gwt_git::MergeTarget::Develop,
                "origin/develop",
            )),
        )]);

        let hydrated = hydrate_branch_entries(entries, &HashSet::new(), &cleanup_targets, true);

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

    fn local_entry(name: &str, is_head: bool) -> BranchListEntry {
        BranchListEntry {
            name: name.to_string(),
            scope: BranchScope::Local,
            is_head,
            upstream: None,
            ahead: 0,
            behind: 0,
            last_commit_date: Some("2026-05-20 08:30:00 +0000".to_string()),
            cleanup_ready: false,
            cleanup: BranchCleanupInfo::default(),
            resume: BranchResumeInfo::unavailable(),
            start_work_eligibility: None,
        }
    }

    #[test]
    fn protected_local_branch_is_risky_selectable_and_remote_protected() {
        // SPEC-2009 FR-070: local main/develop are selectable for LOCAL cleanup
        // as Risky (not Blocked), so they can be deleted while the remote stays
        // protected.
        let hydrated = hydrate_branch_entries(
            vec![local_entry("develop", false)],
            &HashSet::new(),
            &HashMap::new(),
            true,
        );
        assert_eq!(
            hydrated[0].cleanup.availability,
            BranchCleanupAvailability::Risky
        );
        assert_eq!(hydrated[0].cleanup.blocked_reason, None);
        assert!(hydrated[0]
            .cleanup
            .risks
            .contains(&BranchCleanupRisk::ProtectedBase));
    }

    #[test]
    fn protected_branch_checked_out_stays_blocked() {
        // FR-070: a checked-out protected branch is still Blocked (git refuses
        // to delete a branch checked out in a worktree).
        let hydrated = hydrate_branch_entries(
            vec![local_entry("develop", true)],
            &HashSet::new(),
            &HashMap::new(),
            true,
        );
        assert_eq!(
            hydrated[0].cleanup.availability,
            BranchCleanupAvailability::Blocked
        );
        assert_eq!(
            hydrated[0].cleanup.blocked_reason,
            Some(BranchCleanupBlockedReason::CurrentHead)
        );
    }

    #[test]
    fn protected_branch_with_active_session_stays_blocked() {
        // FR-070: an in-use protected branch is still Blocked.
        let sessions = HashSet::from(["develop".to_string()]);
        let hydrated = hydrate_branch_entries(
            vec![local_entry("develop", false)],
            &sessions,
            &HashMap::new(),
            true,
        );
        assert_eq!(
            hydrated[0].cleanup.availability,
            BranchCleanupAvailability::Blocked
        );
        assert_eq!(
            hydrated[0].cleanup.blocked_reason,
            Some(BranchCleanupBlockedReason::ActiveSession)
        );
    }

    #[test]
    fn bare_repo_symbolic_head_does_not_block_protected_base() {
        // SPEC-2009 FR-070: in a bare repo the symbolic HEAD (e.g. main) is not
        // a real worktree checkout — gwt resolves the default branch from
        // origin/HEAD — so main must be Risky-selectable for LOCAL cleanup, not
        // Blocked as the current HEAD. `head_is_real_checkout = false` models a
        // bare root.
        let hydrated = hydrate_branch_entries(
            vec![local_entry("main", true)],
            &HashSet::new(),
            &HashMap::new(),
            false,
        );
        assert_eq!(
            hydrated[0].cleanup.availability,
            BranchCleanupAvailability::Risky
        );
        assert_eq!(hydrated[0].cleanup.blocked_reason, None);
        assert!(hydrated[0]
            .cleanup
            .risks
            .contains(&BranchCleanupRisk::ProtectedBase));
    }
}
