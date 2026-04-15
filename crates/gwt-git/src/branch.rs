//! Branch information and tracking

use std::{collections::HashSet, path::Path};

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

/// Branches that must never be deleted by Branch Cleanup (FR-018b).
///
/// Mirrors the protected list from the current Branch Cleanup contract.
const PROTECTED_BRANCHES: &[&str] = &["main", "master", "develop"];

/// Returns true when `name` matches one of the hard-coded protected branches
/// (FR-018b). Comparisons strip a leading `origin/` so a remote tracking ref
/// referenced via its full name is also recognized.
pub fn is_protected_branch(name: &str) -> bool {
    let bare = name.strip_prefix("origin/").unwrap_or(name);
    PROTECTED_BRANCHES.contains(&bare)
}

/// Where a cleanable branch was determined to be merged into (FR-018a).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeTarget {
    /// Branch is merged into `main` / `master`.
    Main,
    /// Branch is merged into `develop`.
    Develop,
    /// Branch's upstream tracking ref is `[gone]`.
    Gone,
}

impl MergeTarget {
    /// Human-readable label used by the Cleanup confirm modal.
    pub fn label(self) -> &'static str {
        match self {
            Self::Main => "merged → main",
            Self::Develop => "merged → develop",
            Self::Gone => "gone",
        }
    }
}

/// Returns true when every commit reachable from `branch` is also reachable
/// from `base` (FR-018a). Uses `git cherry`, which tolerates squash and rebase
/// merges by comparing patch IDs.
///
/// `base` and `branch` must both resolve in `repo_path`. If `base` does not
/// exist the call returns `Ok(false)` so callers can iterate over multiple
/// candidate bases without having to pre-check each one.
pub fn is_branch_merged_into(repo_path: &Path, branch: &str, base: &str) -> Result<bool> {
    if !ref_exists(repo_path, base)? {
        return Ok(false);
    }
    if !ref_exists(repo_path, branch)? {
        return Ok(false);
    }

    let output = std::process::Command::new("git")
        .args(["cherry", base, branch])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("cherry: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!("cherry {base} {branch}: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_cherry_output(&stdout))
}

/// Walks `bases` in order and returns the first base that already contains
/// every commit on `branch` (FR-018a). When `gone_branches` reports `branch`
/// as having a `[gone]` upstream and no positive merge match was found, the
/// function returns `Some(MergeTarget::Gone)` so callers can still treat the
/// branch as cleanable.
pub fn detect_cleanable_target(
    repo_path: &Path,
    branch: &str,
    bases: &[(&str, MergeTarget)],
    gone_branches: &HashSet<String>,
) -> Result<Option<MergeTarget>> {
    for (base, target) in bases {
        if is_branch_merged_into(repo_path, branch, base)? {
            return Ok(Some(*target));
        }
    }
    if gone_branches.contains(branch) {
        return Ok(Some(MergeTarget::Gone));
    }
    Ok(None)
}

/// Returns the set of local branch names whose upstream tracking ref is
/// `[gone]` (FR-018a). Used to flag branches whose remote was deleted but
/// which still exist locally.
pub fn list_gone_branches(repo_path: &Path) -> Result<HashSet<String>> {
    let output = std::process::Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname:short)\t%(upstream:track)",
            "refs/heads/",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("for-each-ref gone: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!("for-each-ref gone: {stderr}")));
    }

    let mut gone = HashSet::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        let Some(name) = parts.first().map(|s| s.trim()) else {
            continue;
        };
        if name.is_empty() {
            continue;
        }
        let track = parts.get(1).copied().unwrap_or("");
        if track.contains("gone") {
            gone.insert(name.to_string());
        }
    }
    Ok(gone)
}

/// Deletes the local branch `name` (FR-018f). When `force` is true the call
/// uses `git branch -D`, otherwise `git branch -d` (which refuses to delete
/// unmerged branches). The function is a no-op when the branch does not
/// exist so cleanup runs remain idempotent against half-cleaned state.
pub fn delete_local_branch(repo_path: &Path, name: &str, force: bool) -> Result<()> {
    if !ref_exists(repo_path, &format!("refs/heads/{name}"))? {
        return Ok(());
    }

    let flag = if force { "-D" } else { "-d" };
    let output = std::process::Command::new("git")
        .args(["branch", flag, name])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("branch {flag}: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!("branch {flag} {name}: {stderr}")));
    }
    Ok(())
}

/// Returns true when `git rev-parse --verify <ref>` succeeds.
fn ref_exists(repo_path: &Path, refname: &str) -> Result<bool> {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--verify", "--quiet", refname])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("rev-parse: {e}")))?;
    Ok(output.status.success())
}

/// Parse `git cherry` output. The branch is fully merged when the output is
/// empty or contains only `-` lines (commits already in the upstream side).
/// Any `+` line indicates an unmerged commit.
fn parse_cherry_output(stdout: &str) -> bool {
    for line in stdout.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('+') {
            return false;
        }
    }
    true
}

/// Ahead/behind divergence between a branch and its upstream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergenceInfo {
    /// Commits ahead of upstream.
    pub ahead: usize,
    /// Commits behind upstream.
    pub behind: usize,
}

/// Compute how far `branch` has diverged from `upstream` using `git rev-list --left-right`.
///
/// Returns `DivergenceInfo { ahead, behind }`. If either ref is missing the
/// command will fail and an error is returned.
pub fn git_divergence(repo_path: &Path, branch: &str, upstream: &str) -> Result<DivergenceInfo> {
    let output = std::process::Command::new("git")
        .args([
            "-C",
            &repo_path.to_string_lossy(),
            "rev-list",
            "--count",
            "--left-right",
            &format!("{branch}...{upstream}"),
        ])
        .output()
        .map_err(|e| GwtError::Git(format!("rev-list --left-right: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!("rev-list --left-right: {stderr}")));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_divergence_output(&stdout)
}

/// Parse the tab-separated output of `git rev-list --count --left-right`.
///
/// Expected format: `"<ahead>\t<behind>\n"`
fn parse_divergence_output(output: &str) -> Result<DivergenceInfo> {
    let trimmed = output.trim();
    let parts: Vec<&str> = trimmed.split('\t').collect();
    if parts.len() != 2 {
        return Err(GwtError::Git(format!(
            "unexpected rev-list output: {trimmed:?}"
        )));
    }
    let ahead = parts[0]
        .parse::<usize>()
        .map_err(|e| GwtError::Git(format!("parse ahead: {e}")))?;
    let behind = parts[1]
        .parse::<usize>()
        .map_err(|e| GwtError::Git(format!("parse behind: {e}")))?;
    Ok(DivergenceInfo { ahead, behind })
}

/// Information about a Git branch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Branch {
    /// Branch name (e.g. "main", "origin/main").
    pub name: String,
    /// Whether this is a local branch.
    pub is_local: bool,
    /// Whether this is a remote-tracking branch.
    pub is_remote: bool,
    /// Whether this branch is currently checked out (HEAD).
    pub is_head: bool,
    /// Upstream tracking branch name (e.g. "origin/main").
    pub upstream: Option<String>,
    /// Commits ahead of upstream.
    pub ahead: u32,
    /// Commits behind upstream.
    pub behind: u32,
    /// ISO 8601 date of the last commit on this branch.
    pub last_commit_date: Option<String>,
}

/// List branches with full tracking info for the repo at `repo_path`.
pub fn list_branches(repo_path: &Path) -> Result<Vec<Branch>> {
    let format =
        "%(refname:short)\t%(HEAD)\t%(upstream:short)\t%(upstream:track)\t%(creatordate:iso8601)";
    let output = std::process::Command::new("git")
        .args(["for-each-ref", &format!("--format={format}"), "refs/heads/"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("for-each-ref: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::Git(format!("for-each-ref: {stderr}")));
    }

    let mut branches = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(b) = parse_branch_line(line) {
            branches.push(b);
        }
    }

    // Also list remote branches
    let remote_output = std::process::Command::new("git")
        .args([
            "for-each-ref",
            "--format=%(refname:short)\t%(creatordate:iso8601)",
            "refs/remotes/",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::Git(format!("for-each-ref remotes: {e}")))?;

    if remote_output.status.success() {
        for line in String::from_utf8_lossy(&remote_output.stdout).lines() {
            let parts: Vec<&str> = line.split('\t').collect();
            if parts.is_empty() || parts[0].is_empty() {
                continue;
            }
            let name = parts[0].to_string();
            // Skip HEAD pointer
            if name.ends_with("/HEAD") {
                continue;
            }
            let date = parts.get(1).map(|s| s.trim().to_string());
            branches.push(Branch {
                name,
                is_local: false,
                is_remote: true,
                is_head: false,
                upstream: None,
                ahead: 0,
                behind: 0,
                last_commit_date: date,
            });
        }
    }

    Ok(branches)
}

/// Parse ahead/behind from the tracking info string like "[ahead 3, behind 2]".
fn parse_ahead_behind(track: &str) -> (u32, u32) {
    let mut ahead = 0u32;
    let mut behind = 0u32;

    if track.contains("ahead") {
        if let Some(n) = track
            .split("ahead ")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|s| s.parse().ok())
        {
            ahead = n;
        }
    }
    if track.contains("behind") {
        if let Some(n) = track
            .split("behind ")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next())
            .and_then(|s| s.parse().ok())
        {
            behind = n;
        }
    }

    (ahead, behind)
}

fn parse_branch_line(line: &str) -> Option<Branch> {
    let parts: Vec<&str> = line.split('\t').collect();
    if parts.is_empty() || parts[0].is_empty() {
        return None;
    }

    let name = parts[0].to_string();
    let is_head = parts.get(1).is_some_and(|s| s.trim() == "*");
    let upstream = parts
        .get(2)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let track = parts.get(3).unwrap_or(&"");
    let (ahead, behind) = parse_ahead_behind(track);
    let last_commit_date = parts
        .get(4)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim().to_string());

    Some(Branch {
        name,
        is_local: true,
        is_remote: false,
        is_head,
        upstream,
        ahead,
        behind,
        last_commit_date,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ahead_behind_both() {
        assert_eq!(parse_ahead_behind("[ahead 3, behind 2]"), (3, 2));
    }

    #[test]
    fn parse_ahead_behind_ahead_only() {
        assert_eq!(parse_ahead_behind("[ahead 5]"), (5, 0));
    }

    #[test]
    fn parse_ahead_behind_behind_only() {
        assert_eq!(parse_ahead_behind("[behind 1]"), (0, 1));
    }

    #[test]
    fn parse_ahead_behind_empty() {
        assert_eq!(parse_ahead_behind(""), (0, 0));
    }

    #[test]
    fn parse_branch_line_full() {
        let line = "main\t*\torigin/main\t[ahead 1]\t2025-01-01 00:00:00 +0000";
        let b = parse_branch_line(line).unwrap();
        assert_eq!(b.name, "main");
        assert!(b.is_head);
        assert_eq!(b.upstream.as_deref(), Some("origin/main"));
        assert_eq!(b.ahead, 1);
        assert_eq!(b.behind, 0);
        assert!(b.last_commit_date.is_some());
    }

    #[test]
    fn parse_branch_line_minimal() {
        let line = "feature\t \t\t\t";
        let b = parse_branch_line(line).unwrap();
        assert_eq!(b.name, "feature");
        assert!(!b.is_head);
        assert!(b.upstream.is_none());
    }

    #[test]
    fn parse_branch_line_empty() {
        assert!(parse_branch_line("").is_none());
    }

    #[test]
    fn parse_divergence_zero() {
        let info = parse_divergence_output("0\t0\n").unwrap();
        assert_eq!(
            info,
            DivergenceInfo {
                ahead: 0,
                behind: 0
            }
        );
    }

    #[test]
    fn parse_divergence_ahead_behind() {
        let info = parse_divergence_output("3\t5\n").unwrap();
        assert_eq!(
            info,
            DivergenceInfo {
                ahead: 3,
                behind: 5
            }
        );
    }

    #[test]
    fn parse_divergence_no_trailing_newline() {
        let info = parse_divergence_output("1\t2").unwrap();
        assert_eq!(
            info,
            DivergenceInfo {
                ahead: 1,
                behind: 2
            }
        );
    }

    #[test]
    fn parse_divergence_invalid_format() {
        assert!(parse_divergence_output("bad").is_err());
    }

    #[test]
    fn parse_divergence_non_numeric() {
        assert!(parse_divergence_output("abc\tdef").is_err());
    }

    // ---------- Branch Cleanup helpers (FR-018) ----------

    fn run(args: &[&str], cwd: &Path) {
        let output = std::process::Command::new("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap_or_else(|e| panic!("git {args:?}: {e}"));
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_named_repo(path: &Path) {
        run(
            &["init", "--initial-branch=main", path.to_str().unwrap()],
            Path::new("."),
        );
        run(&["config", "user.email", "test@example.com"], path);
        run(&["config", "user.name", "Test"], path);
        run(&["commit", "--allow-empty", "-m", "init"], path);
    }

    fn make_commit(path: &Path, file: &str, content: &str, message: &str) {
        std::fs::write(path.join(file), content).unwrap();
        run(&["add", file], path);
        run(&["commit", "-m", message], path);
    }

    #[test]
    fn is_protected_matches_known_branches() {
        assert!(is_protected_branch("main"));
        assert!(is_protected_branch("master"));
        assert!(is_protected_branch("develop"));
        assert!(is_protected_branch("origin/main"));
        assert!(!is_protected_branch("development"));
        assert!(!is_protected_branch("release"));
        assert!(!is_protected_branch("feature/foo"));
        assert!(!is_protected_branch("bugfix/bar"));
        assert!(!is_protected_branch("hotfix/baz"));
    }

    #[test]
    fn is_branch_merged_into_true_for_real_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "feature/x"], repo);
        make_commit(repo, "a.txt", "a", "feat: a");
        run(&["checkout", "main"], repo);
        run(&["merge", "--no-ff", "-m", "merge x", "feature/x"], repo);

        assert!(is_branch_merged_into(repo, "feature/x", "main").unwrap());
    }

    #[test]
    fn is_branch_merged_into_true_for_squash_merge() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "feature/sq"], repo);
        make_commit(repo, "a.txt", "a\n", "feat: a");
        run(&["checkout", "main"], repo);
        run(&["merge", "--squash", "feature/sq"], repo);
        run(&["commit", "-m", "feat: squashed a"], repo);

        // Squash merge: same patch IDs, so cherry shows '-' for the commit.
        assert!(is_branch_merged_into(repo, "feature/sq", "main").unwrap());
    }

    #[test]
    fn is_branch_merged_into_false_for_unmerged() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "feature/unmerged"], repo);
        make_commit(repo, "a.txt", "unmerged", "feat: unmerged");

        assert!(!is_branch_merged_into(repo, "feature/unmerged", "main").unwrap());
    }

    #[test]
    fn is_branch_merged_into_returns_false_for_missing_base() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "feature/x"], repo);
        make_commit(repo, "a.txt", "a", "feat: a");

        // origin/develop does not exist in this bare-init repo.
        assert!(!is_branch_merged_into(repo, "feature/x", "origin/develop").unwrap());
    }

    #[test]
    fn detect_cleanable_target_walks_bases_in_order() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "develop"], repo);
        run(&["checkout", "-b", "feature/d"], repo);
        make_commit(repo, "d.txt", "d", "feat: d");
        run(&["checkout", "develop"], repo);
        run(&["merge", "--no-ff", "-m", "merge d", "feature/d"], repo);

        let bases = [
            ("main", MergeTarget::Main),
            ("develop", MergeTarget::Develop),
        ];
        let gone = HashSet::new();
        assert_eq!(
            detect_cleanable_target(repo, "feature/d", &bases, &gone).unwrap(),
            Some(MergeTarget::Develop)
        );
    }

    #[test]
    fn detect_cleanable_target_returns_none_when_unmerged() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "feature/free"], repo);
        make_commit(repo, "f.txt", "f", "feat: f");

        let bases = [("main", MergeTarget::Main)];
        let gone = HashSet::new();
        assert_eq!(
            detect_cleanable_target(repo, "feature/free", &bases, &gone).unwrap(),
            None
        );
    }

    #[test]
    fn detect_cleanable_target_falls_back_to_gone() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "feature/abandoned"], repo);
        make_commit(repo, "a.txt", "a", "feat: a");

        let bases = [("main", MergeTarget::Main)];
        let mut gone = HashSet::new();
        gone.insert("feature/abandoned".to_string());
        assert_eq!(
            detect_cleanable_target(repo, "feature/abandoned", &bases, &gone).unwrap(),
            Some(MergeTarget::Gone)
        );
    }

    #[test]
    fn list_gone_branches_returns_empty_for_clean_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "feature/clean"], repo);
        make_commit(repo, "a.txt", "a", "feat: a");

        let gone = list_gone_branches(repo).unwrap();
        assert!(gone.is_empty());
    }

    #[test]
    fn delete_local_branch_force_removes_unmerged_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        run(&["checkout", "-b", "feature/del"], repo);
        make_commit(repo, "a.txt", "a", "feat: a");
        run(&["checkout", "main"], repo);

        delete_local_branch(repo, "feature/del", true).unwrap();

        let branches = list_branches(repo).unwrap();
        assert!(!branches
            .iter()
            .any(|b| b.name == "feature/del" && b.is_local));
    }

    #[test]
    fn delete_local_branch_is_noop_for_missing_branch() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path();
        init_named_repo(repo);
        // Should not error even though the branch does not exist.
        delete_local_branch(repo, "feature/nope", true).unwrap();
    }

    #[test]
    fn parse_cherry_output_treats_minus_lines_as_merged() {
        assert!(parse_cherry_output(""));
        assert!(parse_cherry_output(
            "- abc123 commit message\n- def456 another\n"
        ));
        assert!(!parse_cherry_output("+ abc123 unmerged\n"));
        assert!(!parse_cherry_output("- abc123 ok\n+ def456 unmerged\n"));
    }

    // ----------------------------------------------------

    #[test]
    fn list_branches_in_test_repo() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path();
        std::process::Command::new("git")
            .args(["init", path.to_str().unwrap()])
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "--allow-empty", "-m", "init"])
            .current_dir(path)
            .output()
            .unwrap();

        let branches = list_branches(path).unwrap();
        assert!(!branches.is_empty());
        assert!(branches.iter().any(|b| b.is_local));
    }
}
