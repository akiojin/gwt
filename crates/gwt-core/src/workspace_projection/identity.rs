//! Canonical Work identity: stable Work IDs, grouping keys, and the
//! branch / worktree spelling normalization they are derived from.

use std::{fs, path::Path};

use sha2::{Digest, Sha256};

use crate::paths::project_scope_hash;

use super::*;

pub fn canonical_work_id(
    project_root: &Path,
    branch: Option<&str>,
    worktree_path: Option<&Path>,
) -> Option<String> {
    let branch = branch.map(str::trim).filter(|value| !value.is_empty());
    let (slug_source, identity_kind, identity_value) = if let Some(branch) = branch {
        let identity = canonical_work_branch_identity(branch);
        (identity.clone(), "branch", identity)
    } else {
        let worktree_path = worktree_path?;
        let identity = canonical_worktree_identity(worktree_path);
        if identity.trim().is_empty() {
            return None;
        }
        let slug_source = worktree_path
            .file_name()
            .and_then(|value| value.to_str())
            .filter(|value| !value.trim().is_empty())
            .unwrap_or("worktree");
        (slug_source.to_string(), "worktree", identity)
    };

    let project_hash = project_scope_hash(project_root);
    let mut hasher = Sha256::new();
    hasher.update(project_hash.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(identity_kind.as_bytes());
    hasher.update(b"\0");
    hasher.update(identity_value.as_bytes());
    let digest = hasher.finalize();
    let hex_full = hex::encode(digest);
    Some(format!(
        "work-{}-{}",
        canonical_work_slug(&slug_source),
        &hex_full[..8]
    ))
}

/// One stable successor identity per predecessor, shared by concurrent launches
/// and retries without changing the canonical branch grouping key.
pub fn successor_work_id(predecessor_id: &str) -> String {
    format!(
        "work-successor-{}",
        hex::encode(Sha256::digest(predecessor_id.as_bytes()))
    )
}

/// Canonicalize a stored Work owner onto the durable `SPEC-<n>` spelling.
///
/// `Issue #<n>` and the legacy `SPEC #<n>` spelling may upgrade to the same
/// SPEC owner. The transition is one-way and requires canonical numbers.
pub fn can_upgrade_work_owner(stored: Option<&str>, durable: Option<&str>) -> bool {
    let Some(stored) = stored else {
        return false;
    };
    let Some(durable) = durable else {
        return false;
    };
    let Some((prefix, stored_number)) = ["Issue #", "SPEC #"].into_iter().find_map(|prefix| {
        stored
            .strip_prefix(prefix)
            .and_then(|number| number.parse::<u64>().ok())
            .map(|number| (prefix, number))
    }) else {
        return false;
    };
    let Some(durable_number) = durable
        .strip_prefix("SPEC-")
        .and_then(|number| number.parse::<u64>().ok())
    else {
        return false;
    };
    stored == format!("{prefix}{stored_number}")
        && durable == format!("SPEC-{durable_number}")
        && stored_number == durable_number
}

/// SPEC-2359 W16-2 (FR-389): the Workspace grouping key for one Work item —
/// derived at view-assembly time, never stored (plan decision 6). Works that
/// share a canonical branch (any spelling: `X`, `origin/X`,
/// `refs/remotes/origin/X`) group under one Workspace row; worktree-only
/// items key on the canonical worktree identity; everything else (legacy
/// `workspace-<millis>` / bare-UUID items without containers) keeps its own
/// `item.id` as the key so old rows never vanish.
pub fn workspace_group_key_for_item(project_root: &Path, item: &WorkItem) -> String {
    let branch = item
        .execution_containers
        .iter()
        .find_map(|container| container.branch.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(key) = canonical_work_id(project_root, branch, None) {
        return key;
    }
    let worktree = item
        .execution_containers
        .iter()
        .find_map(|container| container.worktree_path.as_deref());
    if let Some(key) = canonical_work_id(project_root, None, worktree) {
        return key;
    }
    item.id.clone()
}

/// The durable Work owner a branch already names, or `None` when it names no
/// Issue.
///
/// Issue #4479: the worktree scan materializes a Work from a branch alone, and
/// leaving `owner` unset there produced records whose container pointed at
/// `work/issue-<n>` while `owner` stayed null. `workspace.ensure` then read the
/// absent owner as a competing claim and refused with `stored=<none>`, which no
/// operation could repair.
///
/// `work/issue-<n>` is minted by the launch wizard
/// (`knowledge_launch_target_branch_name`), so this is that function's inverse
/// rather than a heuristic: only the exact shape resolves, and every other
/// branch — including a decorated variant like `work/issue-42-followup` — stays
/// ownerless rather than inventing a claim on an Issue.
pub fn work_owner_for_branch(branch: &str) -> Option<String> {
    let branch = branch.trim();
    if branch.is_empty() {
        return None;
    }
    let identity = canonical_work_branch_identity(branch);
    let digits = identity.strip_prefix("work/issue-")?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(format!("Issue #{}", digits.parse::<u64>().ok()?))
}

pub(super) fn canonical_work_branch_identity(branch: &str) -> String {
    if let Some(name) = branch.strip_prefix("refs/remotes/") {
        return name.strip_prefix("origin/").unwrap_or(name).to_string();
    }
    branch.strip_prefix("origin/").unwrap_or(branch).to_string()
}

pub(super) fn canonical_worktree_identity(path: &Path) -> String {
    fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .replace('\\', "/")
}

fn canonical_work_slug(value: &str) -> String {
    let mut slug = String::new();
    let mut previous_dash = false;
    for ch in value.chars().flat_map(char::to_lowercase) {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch);
            previous_dash = false;
        } else if !previous_dash && !slug.is_empty() {
            slug.push('-');
            previous_dash = true;
        }
        if slug.len() >= 48 {
            break;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        "work".to_string()
    } else {
        slug
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Issue #4479 AC-1: a Work whose execution container points at an Issue
    /// branch must never store `owner: null`. `work/issue-<n>` is minted by
    /// `knowledge_launch_target_branch_name`, so reading the owner back out of
    /// it is that function's inverse, not a guess.
    #[test]
    fn work_owner_for_branch_reads_the_issue_back_out_of_a_minted_branch() {
        assert_eq!(
            work_owner_for_branch("work/issue-4477").as_deref(),
            Some("Issue #4477")
        );
        for spelling in [
            "origin/work/issue-4477",
            "refs/remotes/origin/work/issue-4477",
            "  work/issue-4477  ",
        ] {
            assert_eq!(
                work_owner_for_branch(spelling).as_deref(),
                Some("Issue #4477"),
                "{spelling} names the same owner as the bare branch"
            );
        }
    }

    /// Fail-closed: only the exact minted shape yields an owner. Anything else
    /// stays ownerless rather than inventing a claim on an Issue.
    #[test]
    fn work_owner_for_branch_refuses_every_branch_that_is_not_a_minted_issue_branch() {
        for branch in [
            "",
            "   ",
            "develop",
            "work/issue-",
            "work/issue-abc",
            "work/issue-12x",
            "work/issue-4477-followup",
            "feature/issue-4477",
            "work/spec-4477",
        ] {
            assert_eq!(
                work_owner_for_branch(branch),
                None,
                "{branch:?} must not be read as an Issue owner"
            );
        }
    }

    #[test]
    fn workspace_group_key_groups_same_branch_across_spellings_and_ids() {
        let project_root = Path::new("/tmp/repo");
        let now = chrono::Utc::now();
        let mut item_a = WorkItem {
            id: "work-session-aaaa".to_string(),
            title: "a".to_string(),
            intent: None,
            summary: None,
            progress_summary: None,
            status_category: WorkspaceStatusCategory::Active,
            owner: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
            agents: Vec::new(),
            execution_containers: vec![WorkspaceExecutionContainerRef {
                branch: Some("work/x".to_string()),
                worktree_path: None,
                pr_number: None,
                pr_url: None,
                pr_state: None,
            }],
            board_refs: Vec::new(),
            related_work_item_ids: Vec::new(),
            events: Vec::new(),
            legacy_metadata_snapshot: None,
            legacy_metadata_authoritative: false,
            legacy_metadata_snapshot_at: None,
            duplicate_event_containers: Default::default(),
            discarded: false,
            discarded_at: None,
        };
        let mut item_b = item_a.clone();
        item_b.id = "work-session-bbbb".to_string();
        item_b.execution_containers[0].branch = Some("origin/work/x".to_string());
        let mut item_c = item_a.clone();
        item_c.id = "work-x-12345678".to_string();
        item_c.execution_containers[0].branch = Some("refs/remotes/origin/work/x".to_string());

        let key_a = workspace_group_key_for_item(project_root, &item_a);
        let key_b = workspace_group_key_for_item(project_root, &item_b);
        let key_c = workspace_group_key_for_item(project_root, &item_c);
        assert_eq!(key_a, key_b, "origin/X spelling groups with X");
        assert_eq!(key_a, key_c, "refs/remotes/origin/X spelling groups with X");

        // Branchless legacy items keep their own id (adapter: old rows never
        // vanish and never merge into each other).
        item_a.execution_containers.clear();
        item_a.id = "workspace-1748822400000".to_string();
        assert_eq!(
            workspace_group_key_for_item(project_root, &item_a),
            "workspace-1748822400000"
        );
        item_a.id = "0f5e2c1a-aaaa-bbbb-cccc-1234567890ab".to_string();
        assert_eq!(
            workspace_group_key_for_item(project_root, &item_a),
            "0f5e2c1a-aaaa-bbbb-cccc-1234567890ab"
        );
    }

    #[test]
    fn canonical_work_id_is_stable_for_branch_and_uses_readable_slug() {
        let repo = Path::new("/tmp/gwt/repo");

        let first = super::canonical_work_id(repo, Some("work/20260526-0043"), None)
            .expect("branch-derived work id");
        let second = super::canonical_work_id(repo, Some("work/20260526-0043"), None)
            .expect("branch-derived work id");

        assert_eq!(first, second);
        assert!(first.starts_with("work-work-20260526-0043-"));
        assert_eq!(first.rsplit('-').next().expect("hash").len(), 8);
    }

    #[test]
    fn canonical_work_id_changes_when_branch_or_project_changes() {
        let repo_a = Path::new("/tmp/gwt/repo-a");
        let repo_b = Path::new("/tmp/gwt/repo-b");

        let work_a = super::canonical_work_id(repo_a, Some("work/a"), None).expect("work a");
        let work_b = super::canonical_work_id(repo_a, Some("work/b"), None).expect("work b");
        let same_branch_other_project =
            super::canonical_work_id(repo_b, Some("work/a"), None).expect("work a in repo b");

        assert_ne!(work_a, work_b);
        assert_ne!(work_a, same_branch_other_project);
    }

    #[test]
    fn canonical_work_id_normalizes_remote_branch_names() {
        let repo = Path::new("/tmp/gwt/repo");

        let local = super::canonical_work_id(repo, Some("feature/gui"), None).expect("local id");
        let remote =
            super::canonical_work_id(repo, Some("origin/feature/gui"), None).expect("remote id");
        let ref_remote =
            super::canonical_work_id(repo, Some("refs/remotes/origin/feature/gui"), None)
                .expect("remote ref id");

        assert_eq!(local, remote);
        assert_eq!(local, ref_remote);
    }
}
