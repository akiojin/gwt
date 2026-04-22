use std::collections::HashSet;

use gwt::{
    list_branch_entries, list_branch_entries_with_active_sessions, BranchCleanupAvailability,
    BranchCleanupBlockedReason, BranchCleanupRisk, BranchScope,
};
use tempfile::tempdir;

#[test]
fn list_branch_entries_marks_head_and_returns_local_branches() {
    let dir = tempdir().expect("tempdir");

    run_git(dir.path(), &["init", "-q"]);
    run_git(dir.path(), &["config", "user.name", "PoC Tester"]);
    run_git(dir.path(), &["config", "user.email", "poc@example.com"]);
    std::fs::write(dir.path().join("README.md"), "# demo\n").expect("write readme");
    run_git(dir.path(), &["add", "README.md"]);
    run_git(dir.path(), &["commit", "-qm", "init"]);
    run_git(dir.path(), &["branch", "-M", "main"]);
    run_git(dir.path(), &["branch", "feature/alpha"]);

    let branches = list_branch_entries(dir.path()).expect("branch entries");

    assert!(branches.iter().any(|branch| {
        branch.name == "main" && branch.is_head && branch.scope == BranchScope::Local
    }));
    assert!(branches.iter().any(|branch| {
        branch.name == "feature/alpha" && !branch.is_head && branch.scope == BranchScope::Local
    }));
}

#[test]
fn list_branch_entries_marks_unmerged_local_branch_as_risky_cleanup_candidate() {
    let temp = tempdir().expect("tempdir");
    let origin = temp.path().join("origin.git");
    let repo = temp.path().join("repo");

    run_git(
        temp.path(),
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("origin path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["checkout", "-qb", "feature/unmerged"]);
    std::fs::write(repo.join("feature.txt"), "work\n").expect("write feature");
    run_git(&repo, &["add", "feature.txt"]);
    run_git(&repo, &["commit", "-qm", "feature work"]);
    run_git(&repo, &["push", "-u", "origin", "feature/unmerged"]);
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/unmerged")
        .expect("feature branch");

    assert_eq!(feature.scope, BranchScope::Local);
    assert_eq!(
        feature.cleanup.availability,
        BranchCleanupAvailability::Risky
    );
    assert_eq!(
        feature.cleanup.execution_branch.as_deref(),
        Some("feature/unmerged")
    );
    assert_eq!(feature.cleanup.blocked_reason, None);
    assert!(feature.cleanup.risks.contains(&BranchCleanupRisk::Unmerged));
}

#[test]
fn list_branch_entries_prefers_canonical_remote_merge_target_ref() {
    let temp = tempdir().expect("tempdir");
    let origin = temp.path().join("origin.git");
    let repo = temp.path().join("repo");
    let integrator = temp.path().join("integrator");

    run_git(
        temp.path(),
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("origin path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["checkout", "-qb", "develop"]);
    run_git(&repo, &["push", "-u", "origin", "develop"]);
    run_git(&repo, &["checkout", "-qb", "feature/alpha"]);
    std::fs::write(repo.join("alpha.txt"), "alpha\n").expect("write alpha");
    run_git(&repo, &["add", "alpha.txt"]);
    run_git(&repo, &["commit", "-qm", "alpha"]);
    run_git(&repo, &["push", "-u", "origin", "feature/alpha"]);
    run_git(&repo, &["checkout", "main"]);

    run_git(
        temp.path(),
        &[
            "clone",
            origin.to_str().expect("origin path"),
            integrator.to_str().expect("integrator path"),
        ],
    );
    run_git(&integrator, &["config", "user.name", "PoC Tester"]);
    run_git(&integrator, &["config", "user.email", "poc@example.com"]);
    run_git(
        &integrator,
        &["checkout", "-b", "develop", "origin/develop"],
    );
    run_git(
        &integrator,
        &[
            "merge",
            "--no-ff",
            "-m",
            "merge alpha",
            "origin/feature/alpha",
        ],
    );
    run_git(&integrator, &["push", "origin", "develop"]);

    run_git(&repo, &["fetch", "origin", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/alpha")
        .expect("feature branch");

    assert_eq!(feature.scope, BranchScope::Local);
    assert_eq!(
        feature.cleanup.availability,
        BranchCleanupAvailability::Safe
    );
    let merge_target = feature
        .cleanup
        .merge_target
        .as_ref()
        .expect("canonical merge target");
    assert_eq!(merge_target.kind, gwt_git::MergeTarget::Develop);
    assert_eq!(merge_target.reference, "origin/develop");
}

#[test]
fn list_branch_entries_keeps_no_upstream_branch_risky_even_if_local_base_contains_it() {
    let dir = tempdir().expect("tempdir");

    run_git(dir.path(), &["init", "-q"]);
    init_repo(dir.path());
    run_git(dir.path(), &["checkout", "-qb", "develop"]);
    run_git(dir.path(), &["checkout", "-qb", "feature/merged"]);
    std::fs::write(dir.path().join("merged.txt"), "merged\n").expect("write merged");
    run_git(dir.path(), &["add", "merged.txt"]);
    run_git(dir.path(), &["commit", "-qm", "merged"]);
    run_git(dir.path(), &["checkout", "develop"]);
    run_git(
        dir.path(),
        &["merge", "--no-ff", "-m", "merge feature", "feature/merged"],
    );
    run_git(dir.path(), &["checkout", "main"]);

    let branches =
        list_branch_entries_with_active_sessions(dir.path(), &HashSet::new()).expect("entries");
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/merged")
        .expect("feature branch");

    assert_eq!(feature.scope, BranchScope::Local);
    assert_eq!(
        feature.cleanup.availability,
        BranchCleanupAvailability::Risky
    );
    assert_eq!(feature.cleanup.merge_target, None);
    assert!(feature.cleanup.risks.contains(&BranchCleanupRisk::Unmerged));
}

#[test]
fn list_branch_entries_marks_remote_tracking_row_with_local_counterpart_as_risky() {
    let temp = tempdir().expect("tempdir");
    let remote = temp.path().join("origin.git");
    let repo = temp.path().join("repo");

    run_git(
        temp.path(),
        &["init", "--bare", remote.to_str().expect("remote path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["checkout", "-qb", "feature/alpha"]);
    std::fs::write(repo.join("alpha.txt"), "alpha\n").expect("write alpha");
    run_git(&repo, &["add", "alpha.txt"]);
    run_git(&repo, &["commit", "-qm", "alpha"]);
    run_git(&repo, &["push", "-u", "origin", "feature/alpha"]);
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let remote_entry = branches
        .iter()
        .find(|branch| branch.name == "origin/feature/alpha")
        .expect("remote branch");

    assert_eq!(remote_entry.scope, BranchScope::Remote);
    assert_eq!(
        remote_entry.cleanup.availability,
        BranchCleanupAvailability::Risky
    );
    assert_eq!(
        remote_entry.cleanup.execution_branch.as_deref(),
        Some("feature/alpha")
    );
    assert_eq!(remote_entry.cleanup.blocked_reason, None);
    assert!(remote_entry
        .cleanup
        .risks
        .contains(&BranchCleanupRisk::RemoteTracking));
}

#[test]
fn list_branch_entries_marks_local_and_remote_rows_safe_when_upstream_remote_base_contains_branch()
{
    let temp = tempdir().expect("tempdir");
    let origin = temp.path().join("origin.git");
    let repo = temp.path().join("repo");

    run_git(
        temp.path(),
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("origin path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["checkout", "-qb", "develop"]);
    std::fs::write(repo.join("develop.txt"), "develop\n").expect("write develop");
    run_git(&repo, &["add", "develop.txt"]);
    run_git(&repo, &["commit", "-qm", "develop base"]);
    run_git(&repo, &["push", "-u", "origin", "develop"]);
    run_git(&repo, &["checkout", "-qb", "feature/alpha"]);
    std::fs::write(repo.join("alpha.txt"), "alpha\n").expect("write alpha");
    run_git(&repo, &["add", "alpha.txt"]);
    run_git(&repo, &["commit", "-qm", "alpha"]);
    run_git(&repo, &["push", "-u", "origin", "feature/alpha"]);
    run_git(&repo, &["push", "origin", "HEAD:refs/heads/develop"]);
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let local_entry = branches
        .iter()
        .find(|branch| branch.name == "feature/alpha")
        .expect("local branch");
    let remote_entry = branches
        .iter()
        .find(|branch| branch.name == "origin/feature/alpha")
        .expect("remote branch");

    assert_eq!(
        local_entry.cleanup.availability,
        BranchCleanupAvailability::Safe
    );
    assert_eq!(
        local_entry
            .cleanup
            .merge_target
            .as_ref()
            .map(|target| target.reference.as_str()),
        Some("origin/develop")
    );
    assert!(local_entry.cleanup.risks.is_empty());

    assert_eq!(remote_entry.scope, BranchScope::Remote);
    assert_eq!(
        remote_entry.cleanup.execution_branch.as_deref(),
        Some("feature/alpha")
    );
    assert_eq!(
        remote_entry.cleanup.availability,
        BranchCleanupAvailability::Safe
    );
    assert_eq!(
        remote_entry
            .cleanup
            .merge_target
            .as_ref()
            .map(|target| target.reference.as_str()),
        Some("origin/develop")
    );
    assert!(remote_entry.cleanup.risks.is_empty());
}

#[test]
fn list_branch_entries_keeps_remote_row_risky_when_local_branch_is_behind_upstream() {
    let temp = tempdir().expect("tempdir");
    let origin = temp.path().join("origin.git");
    let repo = temp.path().join("repo");
    let peer = temp.path().join("peer");

    run_git(
        temp.path(),
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("origin path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["checkout", "-qb", "develop"]);
    std::fs::write(repo.join("develop.txt"), "develop\n").expect("write develop");
    run_git(&repo, &["add", "develop.txt"]);
    run_git(&repo, &["commit", "-qm", "develop base"]);
    run_git(&repo, &["push", "-u", "origin", "develop"]);
    run_git(&repo, &["checkout", "-qb", "feature/alpha"]);
    std::fs::write(repo.join("alpha.txt"), "alpha\n").expect("write alpha");
    run_git(&repo, &["add", "alpha.txt"]);
    run_git(&repo, &["commit", "-qm", "alpha"]);
    run_git(&repo, &["push", "-u", "origin", "feature/alpha"]);
    run_git(&repo, &["push", "origin", "HEAD:refs/heads/develop"]);
    run_git(&repo, &["checkout", "main"]);

    run_git(
        temp.path(),
        &[
            "clone",
            "-q",
            origin.to_str().expect("origin path"),
            peer.to_str().expect("peer path"),
        ],
    );
    run_git(&peer, &["config", "user.name", "PoC Tester"]);
    run_git(&peer, &["config", "user.email", "poc@example.com"]);
    run_git(
        &peer,
        &["checkout", "-qb", "feature/alpha", "origin/feature/alpha"],
    );
    std::fs::write(peer.join("remote-only.txt"), "remote\n").expect("write remote-only");
    run_git(&peer, &["add", "remote-only.txt"]);
    run_git(&peer, &["commit", "-qm", "remote only"]);
    run_git(&peer, &["push", "origin", "HEAD:refs/heads/feature/alpha"]);

    run_git(&repo, &["fetch", "origin", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let local_entry = branches
        .iter()
        .find(|branch| branch.name == "feature/alpha")
        .expect("local branch");
    let remote_entry = branches
        .iter()
        .find(|branch| branch.name == "origin/feature/alpha")
        .expect("remote branch");

    assert_eq!(local_entry.behind, 1);
    assert_eq!(
        local_entry.cleanup.availability,
        BranchCleanupAvailability::Safe
    );
    assert_eq!(
        local_entry
            .cleanup
            .merge_target
            .as_ref()
            .map(|target| target.reference.as_str()),
        Some("origin/develop")
    );

    assert_eq!(remote_entry.scope, BranchScope::Remote);
    assert_eq!(
        remote_entry.cleanup.execution_branch.as_deref(),
        Some("feature/alpha")
    );
    assert_eq!(
        remote_entry.cleanup.availability,
        BranchCleanupAvailability::Risky
    );
    assert_eq!(
        remote_entry
            .cleanup
            .merge_target
            .as_ref()
            .map(|target| target.reference.as_str()),
        Some("origin/develop")
    );
    assert!(remote_entry
        .cleanup
        .risks
        .contains(&BranchCleanupRisk::RemoteTracking));
}

#[test]
fn list_branch_entries_blocks_remote_tracking_row_without_local_counterpart() {
    let temp = tempdir().expect("tempdir");
    let remote = temp.path().join("origin.git");
    let repo = temp.path().join("repo");

    run_git(
        temp.path(),
        &["init", "--bare", remote.to_str().expect("remote path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            remote.to_str().expect("remote path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["checkout", "-qb", "feature/alpha"]);
    std::fs::write(repo.join("alpha.txt"), "alpha\n").expect("write alpha");
    run_git(&repo, &["add", "alpha.txt"]);
    run_git(&repo, &["commit", "-qm", "alpha"]);
    run_git(&repo, &["push", "-u", "origin", "feature/alpha"]);
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["branch", "-D", "feature/alpha"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let remote_entry = branches
        .iter()
        .find(|branch| branch.name == "origin/feature/alpha")
        .expect("remote branch");

    assert_eq!(remote_entry.scope, BranchScope::Remote);
    assert_eq!(
        remote_entry.cleanup.availability,
        BranchCleanupAvailability::Blocked
    );
    assert_eq!(remote_entry.cleanup.execution_branch, None);
    assert_eq!(
        remote_entry.cleanup.blocked_reason,
        Some(BranchCleanupBlockedReason::RemoteTrackingWithoutLocal)
    );
}

#[test]
fn list_branch_entries_blocks_remote_tracking_row_when_local_branch_tracks_other_remote() {
    let temp = tempdir().expect("tempdir");
    let origin = temp.path().join("origin.git");
    let upstream = temp.path().join("upstream.git");
    let repo = temp.path().join("repo");

    run_git(
        temp.path(),
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    run_git(
        temp.path(),
        &["init", "--bare", upstream.to_str().expect("upstream path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("origin path"),
        ],
    );
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "upstream",
            upstream.to_str().expect("upstream path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["push", "-u", "upstream", "main"]);
    run_git(&repo, &["checkout", "-qb", "feature/alpha"]);
    std::fs::write(repo.join("alpha.txt"), "alpha\n").expect("write alpha");
    run_git(&repo, &["add", "alpha.txt"]);
    run_git(&repo, &["commit", "-qm", "alpha"]);
    run_git(&repo, &["push", "origin", "HEAD:refs/heads/feature/alpha"]);
    run_git(
        &repo,
        &["push", "-u", "upstream", "HEAD:refs/heads/feature/alpha"],
    );
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);
    run_git(&repo, &["fetch", "upstream", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let origin_entry = branches
        .iter()
        .find(|branch| branch.name == "origin/feature/alpha")
        .expect("origin remote branch");
    let upstream_entry = branches
        .iter()
        .find(|branch| branch.name == "upstream/feature/alpha")
        .expect("upstream remote branch");

    assert_eq!(origin_entry.scope, BranchScope::Remote);
    assert_eq!(
        origin_entry.cleanup.availability,
        BranchCleanupAvailability::Blocked
    );
    assert_eq!(origin_entry.cleanup.execution_branch, None);
    assert_eq!(
        origin_entry.cleanup.blocked_reason,
        Some(BranchCleanupBlockedReason::RemoteTrackingWithoutLocal)
    );
    assert_eq!(
        upstream_entry.cleanup.execution_branch.as_deref(),
        Some("feature/alpha")
    );
}

#[test]
fn list_branch_entries_prefers_execution_upstream_remote_base_over_origin() {
    let temp = tempdir().expect("tempdir");
    let origin = temp.path().join("origin.git");
    let upstream = temp.path().join("upstream.git");
    let repo = temp.path().join("repo");

    run_git(
        temp.path(),
        &["init", "--bare", origin.to_str().expect("origin path")],
    );
    run_git(
        temp.path(),
        &["init", "--bare", upstream.to_str().expect("upstream path")],
    );
    run_git(
        temp.path(),
        &["init", "-q", repo.to_str().expect("repo path")],
    );
    init_repo(&repo);
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "origin",
            origin.to_str().expect("origin path"),
        ],
    );
    run_git(
        &repo,
        &[
            "remote",
            "add",
            "upstream",
            upstream.to_str().expect("upstream path"),
        ],
    );
    run_git(&repo, &["push", "-u", "origin", "main"]);
    run_git(&repo, &["push", "-u", "upstream", "main"]);
    run_git(&repo, &["checkout", "-qb", "develop"]);
    std::fs::write(repo.join("develop.txt"), "develop\n").expect("write develop");
    run_git(&repo, &["add", "develop.txt"]);
    run_git(&repo, &["commit", "-qm", "develop base"]);
    run_git(&repo, &["push", "-u", "origin", "develop"]);
    run_git(&repo, &["push", "-u", "upstream", "develop"]);
    run_git(&repo, &["checkout", "-qb", "feature/alpha"]);
    std::fs::write(repo.join("alpha.txt"), "alpha\n").expect("write alpha");
    run_git(&repo, &["add", "alpha.txt"]);
    run_git(&repo, &["commit", "-qm", "alpha"]);
    run_git(&repo, &["push", "origin", "HEAD:refs/heads/feature/alpha"]);
    run_git(
        &repo,
        &["push", "-u", "upstream", "HEAD:refs/heads/feature/alpha"],
    );
    run_git(&repo, &["push", "upstream", "HEAD:refs/heads/develop"]);
    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["fetch", "origin", "--prune"]);
    run_git(&repo, &["fetch", "upstream", "--prune"]);

    let branches =
        list_branch_entries_with_active_sessions(&repo, &HashSet::new()).expect("entries");
    let local_entry = branches
        .iter()
        .find(|branch| branch.name == "feature/alpha")
        .expect("local branch");
    let upstream_entry = branches
        .iter()
        .find(|branch| branch.name == "upstream/feature/alpha")
        .expect("upstream remote branch");

    assert_eq!(
        local_entry.cleanup.availability,
        BranchCleanupAvailability::Safe
    );
    assert_eq!(
        local_entry
            .cleanup
            .merge_target
            .as_ref()
            .map(|target| target.reference.as_str()),
        Some("upstream/develop")
    );
    assert_eq!(
        upstream_entry.cleanup.availability,
        BranchCleanupAvailability::Safe
    );
    assert_eq!(
        upstream_entry
            .cleanup
            .merge_target
            .as_ref()
            .map(|target| target.reference.as_str()),
        Some("upstream/develop")
    );
}

#[test]
fn list_branch_entries_blocks_active_session_branch_from_cleanup() {
    let dir = tempdir().expect("tempdir");

    run_git(dir.path(), &["init", "-q"]);
    init_repo(dir.path());
    run_git(dir.path(), &["checkout", "-qb", "feature/live"]);
    std::fs::write(dir.path().join("live.txt"), "live\n").expect("write live");
    run_git(dir.path(), &["add", "live.txt"]);
    run_git(dir.path(), &["commit", "-qm", "live"]);
    run_git(dir.path(), &["checkout", "main"]);

    let branches = list_branch_entries_with_active_sessions(
        dir.path(),
        &HashSet::from([String::from("feature/live")]),
    )
    .expect("entries");
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/live")
        .expect("feature branch");

    assert_eq!(
        feature.cleanup.availability,
        BranchCleanupAvailability::Blocked
    );
    assert_eq!(
        feature.cleanup.blocked_reason,
        Some(BranchCleanupBlockedReason::ActiveSession)
    );
}

#[test]
fn list_branch_entries_marks_manual_local_branch_without_upstream_as_risky() {
    let dir = tempdir().expect("tempdir");

    run_git(dir.path(), &["init", "-q"]);
    init_repo(dir.path());
    run_git(dir.path(), &["checkout", "-qb", "develop"]);
    std::fs::write(dir.path().join("develop.txt"), "develop\n").expect("write develop");
    run_git(dir.path(), &["add", "develop.txt"]);
    run_git(dir.path(), &["commit", "-qm", "develop base"]);
    run_git(dir.path(), &["checkout", "-qb", "feature/manual"]);
    std::fs::write(dir.path().join("manual.txt"), "manual\n").expect("write manual");
    run_git(dir.path(), &["add", "manual.txt"]);
    run_git(dir.path(), &["commit", "-qm", "manual"]);
    run_git(dir.path(), &["checkout", "develop"]);
    run_git(
        dir.path(),
        &["merge", "--no-ff", "-m", "merge manual", "feature/manual"],
    );
    run_git(dir.path(), &["checkout", "main"]);

    let branches =
        list_branch_entries_with_active_sessions(dir.path(), &HashSet::new()).expect("entries");
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/manual")
        .expect("feature branch");

    assert_eq!(feature.scope, BranchScope::Local);
    assert_eq!(
        feature.cleanup.availability,
        BranchCleanupAvailability::Risky
    );
    assert_eq!(feature.cleanup.merge_target, None);
    assert!(feature.cleanup.risks.contains(&BranchCleanupRisk::Unmerged));
}

fn init_repo(repo: &std::path::Path) {
    run_git(repo, &["config", "user.name", "PoC Tester"]);
    run_git(repo, &["config", "user.email", "poc@example.com"]);
    std::fs::write(repo.join("README.md"), "# demo\n").expect("write readme");
    run_git(repo, &["add", "README.md"]);
    run_git(repo, &["commit", "-qm", "init"]);
    run_git(repo, &["branch", "-M", "main"]);
}

fn run_git(repo: &std::path::Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}
