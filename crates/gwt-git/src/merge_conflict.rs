//! What a `CONFLICTING` pull request actually conflicts on (SPEC #3835 AC-4).
//!
//! `pr.list` used to report `mergeable: CONFLICTING` and nothing else, so the
//! PM ran `git merge-tree --write-tree --name-only` and `git rev-list --count`
//! by hand on every conflicted row to decide whether the conflict was
//! mechanical or needed the owner. Both answers come from local git, so this
//! measurement spends no GitHub budget (AC-5) and stays truthful even when the
//! PR rows were served from cache.

use std::path::Path;

use gwt_core::process::{resolved_command, ProcessPlanRequest};
use serde::{Deserialize, Serialize};

/// Conflicting paths listed in full before the report degrades to a count.
///
/// A merge whose conflict spans more than this is a "relaunch the owner" answer
/// whatever the file names are, and a 265-commit-behind branch can conflict on
/// hundreds of paths — long enough to drown the digest it is meant to inform.
pub const CONFLICT_FILE_LIST_CAP: usize = 40;

/// The conflict and the drift behind one open PR.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrConflictReport {
    /// Paths that conflict, capped at [`CONFLICT_FILE_LIST_CAP`].
    #[serde(default)]
    pub conflicting_files: Vec<String>,
    /// How many paths conflict in total, including any the list omitted.
    #[serde(default)]
    pub conflicting_file_count: usize,
    /// Whether `conflicting_files` was truncated to the cap.
    #[serde(default)]
    pub files_truncated: bool,
    /// Commits the base has that the head does not. `None` when the count
    /// could not be measured.
    #[serde(default)]
    pub behind_by: Option<u32>,
    /// Why the measurement is incomplete, when it is: `head_ref_missing`,
    /// `base_ref_missing`, or `git_failed`. `None` on a complete measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub probe: Option<String>,
}

impl PrConflictReport {
    fn unmeasurable(probe: &str) -> Self {
        Self {
            probe: Some(probe.to_string()),
            ..Self::default()
        }
    }
}

fn run_git(repo_path: &Path, args: &[&str]) -> Option<(bool, String)> {
    let output = resolved_command(
        ProcessPlanRequest::new("git")
            .args(args)
            .current_dir(repo_path)
            .env("GIT_TERMINAL_PROMPT", "0")
            // A measurement must never reach the network: these refs are
            // already local, and a lazy fetch would turn a free probe into a
            // multi-second stall inside the PM's cycle.
            .env("GIT_NO_LAZY_FETCH", "1"),
    )
    .ok()?
    .output()
    .ok()?;
    Some((
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    ))
}

fn rev_exists(repo_path: &Path, rev: &str) -> bool {
    run_git(repo_path, &["rev-parse", "--verify", "--quiet", rev])
        .is_some_and(|(success, _)| success)
}

/// Commits `base` has that `head` does not.
fn behind_count(repo_path: &Path, base: &str, head: &str) -> Option<u32> {
    let range = format!("{head}..{base}");
    let (success, stdout) = run_git(repo_path, &["rev-list", "--count", &range])?;
    success.then(|| stdout.trim().parse().ok())?
}

/// Paths that conflict when `head` merges into `base`.
///
/// `git merge-tree --write-tree` exits non-zero on conflict. Its first section
/// is the merged tree OID followed by the conflicted paths (one per line under
/// `--name-only`); a blank line then separates the human-readable merge
/// messages. A clean merge exits zero and prints the tree OID alone.
fn conflicting_paths(repo_path: &Path, base: &str, head: &str) -> Option<Vec<String>> {
    let (success, stdout) = run_git(
        repo_path,
        &["merge-tree", "--write-tree", "--name-only", base, head],
    )?;
    if success {
        return Some(Vec::new());
    }
    let mut paths: Vec<String> = stdout
        .split("\n\n")
        .next()
        .unwrap_or("")
        .lines()
        // Drop the merged tree OID that heads the section.
        .skip(1)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect();
    paths.sort();
    paths.dedup();
    Some(paths)
}

/// Measure the conflict between a PR head and its base using local refs only.
///
/// Callers run this exclusively for a PR GitHub already reported as
/// `CONFLICTING` (SPEC #3835 AC-5): `merge-tree` is cheap per PR but not free
/// across an inventory, and a clean PR has nothing to report.
pub fn measure_pr_conflict(repo_path: &Path, base_ref: &str, head_ref: &str) -> PrConflictReport {
    if !rev_exists(repo_path, base_ref) {
        return PrConflictReport::unmeasurable("base_ref_missing");
    }
    if !rev_exists(repo_path, head_ref) {
        return PrConflictReport::unmeasurable("head_ref_missing");
    }
    let behind_by = behind_count(repo_path, base_ref, head_ref);
    let Some(paths) = conflicting_paths(repo_path, base_ref, head_ref) else {
        return PrConflictReport {
            behind_by,
            ..PrConflictReport::unmeasurable("git_failed")
        };
    };
    let conflicting_file_count = paths.len();
    let files_truncated = conflicting_file_count > CONFLICT_FILE_LIST_CAP;
    let mut conflicting_files = paths;
    if files_truncated {
        conflicting_files.truncate(CONFLICT_FILE_LIST_CAP);
    }
    PrConflictReport {
        conflicting_files,
        conflicting_file_count,
        files_truncated,
        behind_by,
        probe: None,
    }
}

/// The remote-tracking ref a branch name resolves to for this measurement.
pub fn remote_tracking_ref(branch: &str) -> String {
    if branch.starts_with("origin/") || branch.starts_with("refs/") {
        branch.to_string()
    } else {
        format!("origin/{branch}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn git(repo: &Path, args: &[&str]) {
        let output = resolved_command(ProcessPlanRequest::new("git").args(args).current_dir(repo))
            .expect("resolve git")
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn write(repo: &Path, name: &str, contents: &str) {
        std::fs::write(repo.join(name), contents).expect("write fixture file");
    }

    /// Two branches that conflict on `a.txt` while the base is two commits
    /// ahead of the head. The branches avoid the name `head`, which collides
    /// with `HEAD` on a case-insensitive filesystem.
    fn conflicting_repo() -> TempDir {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path();
        git(repo, &["init", "--initial-branch=mainline"]);
        git(repo, &["config", "user.email", "t@example.com"]);
        git(repo, &["config", "user.name", "T"]);
        write(repo, "a.txt", "original\n");
        write(repo, "b.txt", "shared\n");
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "root"]);

        git(repo, &["checkout", "-b", "topic"]);
        write(repo, "a.txt", "topic side\n");
        git(repo, &["commit", "-am", "topic edit"]);

        git(repo, &["checkout", "mainline"]);
        write(repo, "a.txt", "mainline side\n");
        git(repo, &["commit", "-am", "mainline edit"]);
        write(repo, "c.txt", "new\n");
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "mainline second"]);
        tmp
    }

    /// SPEC #3835 AC-4: the conflicted paths and the drift come back together,
    /// so the PM never has to run git to decide who owns the conflict.
    #[test]
    fn conflicting_files_and_behind_count_come_from_local_git() {
        let tmp = conflicting_repo();
        let report = measure_pr_conflict(tmp.path(), "mainline", "topic");
        assert_eq!(report.conflicting_files, vec!["a.txt".to_string()]);
        assert_eq!(report.conflicting_file_count, 1);
        assert!(!report.files_truncated);
        assert_eq!(report.behind_by, Some(2), "base is two commits ahead");
        assert_eq!(report.probe, None);
    }

    /// A merge that is actually clean reports no conflicting path, so a stale
    /// `CONFLICTING` from GitHub cannot invent one.
    #[test]
    fn a_clean_merge_reports_no_conflicting_files() {
        let tmp = conflicting_repo();
        let report = measure_pr_conflict(tmp.path(), "mainline", "mainline");
        assert!(report.conflicting_files.is_empty());
        assert_eq!(report.conflicting_file_count, 0);
        assert_eq!(report.behind_by, Some(0));
    }

    /// A missing ref is reported as unmeasured, never as "no conflict".
    #[test]
    fn a_missing_ref_is_unmeasured_rather_than_clean() {
        let tmp = conflicting_repo();
        assert_eq!(
            measure_pr_conflict(tmp.path(), "mainline", "work/issue-nope").probe,
            Some("head_ref_missing".to_string())
        );
        assert_eq!(
            measure_pr_conflict(tmp.path(), "no/such/base", "topic").probe,
            Some("base_ref_missing".to_string())
        );
    }

    /// SPEC #3835 T-023: a conflict wider than the cap degrades to a count so
    /// one row cannot drown the digest.
    #[test]
    fn a_wide_conflict_degrades_to_a_count() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path();
        git(repo, &["init", "--initial-branch=mainline"]);
        git(repo, &["config", "user.email", "t@example.com"]);
        git(repo, &["config", "user.name", "T"]);
        let wide = CONFLICT_FILE_LIST_CAP + 5;
        for index in 0..wide {
            write(repo, &format!("f{index:03}.txt"), "original\n");
        }
        git(repo, &["add", "."]);
        git(repo, &["commit", "-m", "root"]);

        git(repo, &["checkout", "-b", "topic"]);
        for index in 0..wide {
            write(repo, &format!("f{index:03}.txt"), "topic\n");
        }
        git(repo, &["commit", "-am", "topic edits"]);

        git(repo, &["checkout", "mainline"]);
        for index in 0..wide {
            write(repo, &format!("f{index:03}.txt"), "mainline\n");
        }
        git(repo, &["commit", "-am", "mainline edits"]);

        let report = measure_pr_conflict(repo, "mainline", "topic");
        assert_eq!(report.conflicting_file_count, wide);
        assert!(report.files_truncated);
        assert_eq!(report.conflicting_files.len(), CONFLICT_FILE_LIST_CAP);
    }

    #[test]
    fn branch_names_resolve_to_their_remote_tracking_ref() {
        assert_eq!(remote_tracking_ref("work/issue-7"), "origin/work/issue-7");
        assert_eq!(remote_tracking_ref("origin/develop"), "origin/develop");
        assert_eq!(
            remote_tracking_ref("refs/remotes/origin/develop"),
            "refs/remotes/origin/develop"
        );
    }
}
