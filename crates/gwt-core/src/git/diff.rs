//! Git diff and branch comparison operations for GitView

use crate::error::{GwtError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

const DIFF_LINE_LIMIT: usize = 1000;

/// Kind of file change in a diff
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// A changed file in a branch diff
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    pub path: String,
    pub kind: FileChangeKind,
    pub additions: usize,
    pub deletions: usize,
    pub is_binary: bool,
}

/// Diff content for a single file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiff {
    pub content: String,
    pub truncated: bool,
}

/// Commit entry for GitView (distinct from commit::CommitEntry which lacks timestamp/author)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitViewCommit {
    pub sha: String,
    pub message: String,
    pub timestamp: i64,
    pub author: String,
}

/// Working tree entry (staged or unstaged change)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkingTreeEntry {
    pub path: String,
    pub status: FileChangeKind,
    pub is_staged: bool,
}

/// Summary of git changes for a branch
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitChangeSummary {
    pub file_count: usize,
    pub commit_count: usize,
    pub stash_count: usize,
    pub base_branch: String,
}

/// Detect the base branch for comparison by checking upstream, falling back to "main"
pub fn detect_base_branch(repo_path: &Path, branch: &str) -> Result<String> {
    let output = Command::new("git")
        .args([
            "rev-parse",
            "--abbrev-ref",
            &format!("{}@{{upstream}}", branch),
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "detect_base_branch".to_string(),
            details: e.to_string(),
        })?;

    if output.status.success() {
        let upstream = String::from_utf8_lossy(&output.stdout).trim().to_string();
        // Strip remote prefix (e.g., "origin/main" -> "main")
        if let Some(pos) = upstream.find('/') {
            return Ok(upstream[pos + 1..].to_string());
        }
        return Ok(upstream);
    }

    Ok("main".to_string())
}

/// List candidate base branches that exist in the repository
pub fn list_base_branch_candidates(repo_path: &Path) -> Result<Vec<String>> {
    let candidates = ["main", "master", "develop"];
    let mut result = Vec::new();

    for name in &candidates {
        let output = Command::new("git")
            .args(["rev-parse", "--verify", &format!("refs/heads/{}", name)])
            .current_dir(repo_path)
            .output()
            .map_err(|e| GwtError::GitOperationFailed {
                operation: "rev-parse --verify".to_string(),
                details: e.to_string(),
            })?;

        if output.status.success() {
            result.push(name.to_string());
        }
    }

    Ok(result)
}

/// Get changed files between a branch and its base branch
pub fn get_branch_diff_files(
    repo_path: &Path,
    branch: &str,
    base_branch: &str,
) -> Result<Vec<FileChange>> {
    let range = format!("{}..{}", base_branch, branch);

    // Get numstat for additions/deletions and binary detection
    let numstat_output = Command::new("git")
        .args(["diff", "--numstat", &range])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "diff --numstat".to_string(),
            details: e.to_string(),
        })?;

    if !numstat_output.status.success() {
        return Err(GwtError::GitOperationFailed {
            operation: "diff --numstat".to_string(),
            details: String::from_utf8_lossy(&numstat_output.stderr).to_string(),
        });
    }

    // Get name-status for change kind
    let status_output = Command::new("git")
        .args(["diff", "--name-status", &range])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "diff --name-status".to_string(),
            details: e.to_string(),
        })?;

    if !status_output.status.success() {
        return Err(GwtError::GitOperationFailed {
            operation: "diff --name-status".to_string(),
            details: String::from_utf8_lossy(&status_output.stderr).to_string(),
        });
    }

    // Parse numstat: additions\tdeletions\tpath
    let numstat = String::from_utf8_lossy(&numstat_output.stdout);
    let mut stats_map: HashMap<String, (usize, usize, bool)> = HashMap::new();

    for line in numstat.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            let is_binary = parts[0] == "-" && parts[1] == "-";
            let additions = parts[0].parse().unwrap_or(0);
            let deletions = parts[1].parse().unwrap_or(0);
            let path = parts[2].to_string();
            stats_map.insert(path, (additions, deletions, is_binary));
        }
    }

    // Parse name-status: STATUS\tPATH (or STATUS\tOLD\tNEW for renames)
    let status_str = String::from_utf8_lossy(&status_output.stdout);
    let mut files = Vec::new();

    for line in status_str.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 2 {
            continue;
        }

        let status = parts[0];
        let path = if parts.len() >= 3 && status.starts_with('R') {
            parts[2].to_string()
        } else {
            parts[1].to_string()
        };

        let kind = match status.chars().next() {
            Some('A') => FileChangeKind::Added,
            Some('D') => FileChangeKind::Deleted,
            Some('R') => FileChangeKind::Renamed,
            _ => FileChangeKind::Modified,
        };

        let (additions, deletions, is_binary) =
            stats_map.get(&path).copied().unwrap_or((0, 0, false));

        files.push(FileChange {
            path,
            kind,
            additions,
            deletions,
            is_binary,
        });
    }

    Ok(files)
}

/// Get the unified diff content for a single file, truncated at 1000 lines
pub fn get_file_diff(
    repo_path: &Path,
    branch: &str,
    base_branch: &str,
    file_path: &str,
) -> Result<FileDiff> {
    let range = format!("{}..{}", base_branch, branch);
    let output = Command::new("git")
        .args(["diff", &range, "--", file_path])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "diff file".to_string(),
            details: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(GwtError::GitOperationFailed {
            operation: "diff file".to_string(),
            details: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let content = String::from_utf8_lossy(&output.stdout).to_string();

    // Detect binary file
    if content.contains("Binary files") && content.contains("differ") {
        return Ok(FileDiff {
            content: "Binary file changed".to_string(),
            truncated: false,
        });
    }

    // Truncate if exceeds line limit
    let lines: Vec<&str> = content.lines().collect();
    if lines.len() > DIFF_LINE_LIMIT {
        Ok(FileDiff {
            content: lines[..DIFF_LINE_LIMIT].join("\n"),
            truncated: true,
        })
    } else {
        Ok(FileDiff {
            content,
            truncated: false,
        })
    }
}

/// Get working tree status (staged and unstaged changes)
pub fn get_working_tree_status(repo_path: &Path) -> Result<Vec<WorkingTreeEntry>> {
    let output = Command::new("git")
        .args(["status", "--porcelain"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "status --porcelain".to_string(),
            details: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(GwtError::GitOperationFailed {
            operation: "status --porcelain".to_string(),
            details: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();

    for line in stdout.lines() {
        if line.len() < 3 {
            continue;
        }

        let bytes = line.as_bytes();
        let index_status = bytes[0] as char;
        let worktree_status = bytes[1] as char;
        let path = line[3..].to_string();

        // Untracked files
        if index_status == '?' {
            entries.push(WorkingTreeEntry {
                path,
                status: FileChangeKind::Added,
                is_staged: false,
            });
            continue;
        }

        // Staged change
        if index_status != ' ' {
            let status = match index_status {
                'A' => FileChangeKind::Added,
                'D' => FileChangeKind::Deleted,
                'R' => FileChangeKind::Renamed,
                _ => FileChangeKind::Modified,
            };
            entries.push(WorkingTreeEntry {
                path: path.clone(),
                status,
                is_staged: true,
            });
        }

        // Unstaged change
        if worktree_status != ' ' {
            let status = match worktree_status {
                'D' => FileChangeKind::Deleted,
                _ => FileChangeKind::Modified,
            };
            entries.push(WorkingTreeEntry {
                path,
                status,
                is_staged: false,
            });
        }
    }

    Ok(entries)
}

/// Get commits between a branch and its base branch with pagination
pub fn get_branch_commits(
    repo_path: &Path,
    branch: &str,
    base_branch: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<GitViewCommit>> {
    let range = format!("{}..{}", base_branch, branch);
    let output = Command::new("git")
        .args([
            "log",
            &range,
            "--format=%H%x00%s%x00%at%x00%an",
            &format!("--skip={}", offset),
            &format!("-n {}", limit),
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "log".to_string(),
            details: e.to_string(),
        })?;

    if !output.status.success() {
        return Err(GwtError::GitOperationFailed {
            operation: "log".to_string(),
            details: String::from_utf8_lossy(&output.stderr).to_string(),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut commits = Vec::new();

    for line in stdout.lines() {
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.splitn(4, '\0').collect();
        if parts.len() == 4 {
            commits.push(GitViewCommit {
                sha: parts[0].to_string(),
                message: parts[1].to_string(),
                timestamp: parts[2].parse().unwrap_or(0),
                author: parts[3].to_string(),
            });
        }
    }

    Ok(commits)
}

/// Get a summary of git changes (file count, commit count, stash count)
pub fn get_git_change_summary(
    repo_path: &Path,
    branch: &str,
    base_branch: &str,
) -> Result<GitChangeSummary> {
    let range = format!("{}..{}", base_branch, branch);

    // File count via --name-only
    let file_output = Command::new("git")
        .args(["diff", "--name-only", &range])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "diff --name-only".to_string(),
            details: e.to_string(),
        })?;

    let file_count = if file_output.status.success() {
        String::from_utf8_lossy(&file_output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .count()
    } else {
        0
    };

    // Commit count via rev-list --count
    let commit_output = Command::new("git")
        .args(["rev-list", "--count", &range])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "rev-list --count".to_string(),
            details: e.to_string(),
        })?;

    let commit_count = if commit_output.status.success() {
        String::from_utf8_lossy(&commit_output.stdout)
            .trim()
            .parse()
            .unwrap_or(0)
    } else {
        0
    };

    // Stash count
    let stash_output = Command::new("git")
        .args(["stash", "list"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "stash list".to_string(),
            details: e.to_string(),
        })?;

    let stash_count = if stash_output.status.success() {
        String::from_utf8_lossy(&stash_output.stdout)
            .lines()
            .filter(|l| !l.is_empty())
            .count()
    } else {
        0
    };

    Ok(GitChangeSummary {
        file_count,
        commit_count,
        stash_count,
        base_branch: base_branch.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;

    fn run_git(repo_path: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo_path)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn get_current_branch_name(repo_path: &Path) -> String {
        let output = Command::new("git")
            .args(["rev-parse", "--abbrev-ref", "HEAD"])
            .current_dir(repo_path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    fn create_test_repo() -> TempDir {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]);
        run_git(temp.path(), &["config", "user.email", "test@test.com"]);
        run_git(temp.path(), &["config", "user.name", "Test User"]);
        std::fs::write(temp.path().join("README.md"), "# Test\n").unwrap();
        std::fs::write(temp.path().join("lib.rs"), "fn main() {}\n").unwrap();
        std::fs::write(temp.path().join("old.rs"), "// old file\n").unwrap();
        run_git(temp.path(), &["add", "."]);
        run_git(temp.path(), &["commit", "-m", "initial commit"]);
        temp
    }

    /// Create a repo with a feature branch that has 3 file changes and 3 commits
    fn create_repo_with_feature(temp: &TempDir) -> String {
        let base = get_current_branch_name(temp.path());

        run_git(temp.path(), &["checkout", "-b", "feature"]);

        // Commit 1: add new file
        std::fs::write(temp.path().join("new.rs"), "// new file\nfn new() {}\n").unwrap();
        run_git(temp.path(), &["add", "new.rs"]);
        run_git(temp.path(), &["commit", "-m", "add new.rs"]);

        // Commit 2: modify file (add lines, remove original)
        std::fs::write(
            temp.path().join("lib.rs"),
            "fn main() {\n    println!(\"hello\");\n    println!(\"world\");\n}\n",
        )
        .unwrap();
        run_git(temp.path(), &["add", "lib.rs"]);
        run_git(temp.path(), &["commit", "-m", "modify lib.rs"]);

        // Commit 3: delete file
        std::fs::remove_file(temp.path().join("old.rs")).unwrap();
        run_git(temp.path(), &["add", "old.rs"]);
        run_git(temp.path(), &["commit", "-m", "delete old.rs"]);

        base
    }

    // T-DIFF-001: Basic branch diff file retrieval
    #[test]
    fn test_get_branch_diff_files_basic() {
        let temp = create_test_repo();
        let base = create_repo_with_feature(&temp);

        let files = get_branch_diff_files(temp.path(), "feature", &base).unwrap();
        assert_eq!(files.len(), 3);

        for file in &files {
            assert!(!file.path.is_empty());
        }
    }

    // T-DIFF-002: File addition detection
    #[test]
    fn test_get_branch_diff_files_added() {
        let temp = create_test_repo();
        let base = create_repo_with_feature(&temp);

        let files = get_branch_diff_files(temp.path(), "feature", &base).unwrap();
        let new_file = files.iter().find(|f| f.path == "new.rs").unwrap();
        assert_eq!(new_file.kind, FileChangeKind::Added);
        assert!(new_file.additions > 0);
        assert_eq!(new_file.deletions, 0);
        assert!(!new_file.is_binary);
    }

    // T-DIFF-003: File deletion detection
    #[test]
    fn test_get_branch_diff_files_deleted() {
        let temp = create_test_repo();
        let base = create_repo_with_feature(&temp);

        let files = get_branch_diff_files(temp.path(), "feature", &base).unwrap();
        let deleted = files.iter().find(|f| f.path == "old.rs").unwrap();
        assert_eq!(deleted.kind, FileChangeKind::Deleted);
        assert_eq!(deleted.additions, 0);
        assert!(deleted.deletions > 0);
        assert!(!deleted.is_binary);
    }

    // T-DIFF-004: File modification detection
    #[test]
    fn test_get_branch_diff_files_modified() {
        let temp = create_test_repo();
        let base = create_repo_with_feature(&temp);

        let files = get_branch_diff_files(temp.path(), "feature", &base).unwrap();
        let modified = files.iter().find(|f| f.path == "lib.rs").unwrap();
        assert_eq!(modified.kind, FileChangeKind::Modified);
        assert!(modified.additions > 0);
        assert!(modified.deletions > 0);
        assert!(!modified.is_binary);
    }

    // T-DIFF-005: Binary file detection
    #[test]
    fn test_get_branch_diff_files_binary() {
        let temp = create_test_repo();
        let base = get_current_branch_name(temp.path());

        run_git(temp.path(), &["checkout", "-b", "feature-bin"]);
        // Write binary content with NUL bytes (git uses NUL to detect binary)
        let mut binary_data = vec![0x89, 0x50, 0x4E, 0x47, 0x00, 0x00, 0x00, 0x0D];
        binary_data.extend_from_slice(&[0x00; 64]);
        std::fs::write(temp.path().join("image.png"), &binary_data).unwrap();
        run_git(temp.path(), &["add", "image.png"]);
        run_git(temp.path(), &["commit", "-m", "add binary"]);

        let files = get_branch_diff_files(temp.path(), "feature-bin", &base).unwrap();
        let binary = files.iter().find(|f| f.path == "image.png").unwrap();
        assert_eq!(binary.kind, FileChangeKind::Added);
        assert!(binary.is_binary);
    }

    // T-DIFF-006: No changes branch
    #[test]
    fn test_get_branch_diff_files_no_changes() {
        let temp = create_test_repo();
        let base = get_current_branch_name(temp.path());

        run_git(temp.path(), &["checkout", "-b", "feature-empty"]);
        // No commits on feature branch

        let files = get_branch_diff_files(temp.path(), "feature-empty", &base).unwrap();
        assert!(files.is_empty());
    }

    // T-DIFF-010: Basic file diff retrieval
    #[test]
    fn test_get_file_diff_basic() {
        let temp = create_test_repo();
        let base = create_repo_with_feature(&temp);

        let diff = get_file_diff(temp.path(), "feature", &base, "lib.rs").unwrap();
        assert!(!diff.content.is_empty());
        assert!(!diff.truncated);
    }

    // T-DIFF-011: Large diff truncation
    #[test]
    fn test_get_file_diff_truncation() {
        let temp = create_test_repo();
        let base = get_current_branch_name(temp.path());

        run_git(temp.path(), &["checkout", "-b", "feature-large"]);

        // Create a file with >1000 lines of changes
        let mut content = String::new();
        for i in 0..2000 {
            content.push_str(&format!("line {} of large file\n", i));
        }
        std::fs::write(temp.path().join("large.rs"), &content).unwrap();
        run_git(temp.path(), &["add", "large.rs"]);
        run_git(temp.path(), &["commit", "-m", "add large file"]);

        let diff = get_file_diff(temp.path(), "feature-large", &base, "large.rs").unwrap();
        assert!(diff.truncated);
        let line_count = diff.content.lines().count();
        assert!(line_count <= DIFF_LINE_LIMIT);
    }

    // T-DIFF-012: Binary file diff
    #[test]
    fn test_get_file_diff_binary() {
        let temp = create_test_repo();
        let base = get_current_branch_name(temp.path());

        run_git(temp.path(), &["checkout", "-b", "feature-bin2"]);
        std::fs::write(
            temp.path().join("data.bin"),
            [0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD],
        )
        .unwrap();
        run_git(temp.path(), &["add", "data.bin"]);
        run_git(temp.path(), &["commit", "-m", "add binary data"]);

        let diff = get_file_diff(temp.path(), "feature-bin2", &base, "data.bin").unwrap();
        assert_eq!(diff.content, "Binary file changed");
        assert!(!diff.truncated);
    }

    // T-DIFF-020: Staged file detection
    #[test]
    fn test_working_tree_staged() {
        let temp = create_test_repo();

        std::fs::write(temp.path().join("staged.rs"), "// staged\n").unwrap();
        run_git(temp.path(), &["add", "staged.rs"]);

        let entries = get_working_tree_status(temp.path()).unwrap();
        let staged = entries
            .iter()
            .find(|e| e.path == "staged.rs" && e.is_staged)
            .unwrap();
        assert_eq!(staged.status, FileChangeKind::Added);
        assert!(staged.is_staged);
    }

    // T-DIFF-021: Unstaged file detection
    #[test]
    fn test_working_tree_unstaged() {
        let temp = create_test_repo();

        // Modify an existing tracked file without staging
        std::fs::write(temp.path().join("lib.rs"), "fn modified() {}\n").unwrap();

        let entries = get_working_tree_status(temp.path()).unwrap();
        let unstaged = entries
            .iter()
            .find(|e| e.path == "lib.rs" && !e.is_staged)
            .unwrap();
        assert_eq!(unstaged.status, FileChangeKind::Modified);
        assert!(!unstaged.is_staged);
    }

    // T-DIFF-022: Clean working tree
    #[test]
    fn test_working_tree_clean() {
        let temp = create_test_repo();
        let entries = get_working_tree_status(temp.path()).unwrap();
        assert!(entries.is_empty());
    }

    // T-DIFF-030: Basic commit retrieval
    #[test]
    fn test_get_branch_commits_basic() {
        let temp = create_test_repo();
        let base = create_repo_with_feature(&temp);

        let commits = get_branch_commits(temp.path(), "feature", &base, 0, 20).unwrap();
        assert_eq!(commits.len(), 3);

        for commit in &commits {
            assert!(!commit.sha.is_empty());
            assert!(!commit.message.is_empty());
            assert!(commit.timestamp > 0);
            assert!(!commit.author.is_empty());
        }
    }

    // T-DIFF-031: Pagination (offset/limit)
    #[test]
    fn test_get_branch_commits_pagination() {
        let temp = create_test_repo();
        let base = create_repo_with_feature(&temp);

        // Get first 2
        let page1 = get_branch_commits(temp.path(), "feature", &base, 0, 2).unwrap();
        assert_eq!(page1.len(), 2);

        // Get remaining
        let page2 = get_branch_commits(temp.path(), "feature", &base, 2, 2).unwrap();
        assert_eq!(page2.len(), 1);

        // No overlap
        assert_ne!(page1[0].sha, page2[0].sha);
    }

    // T-DIFF-032: Zero commits
    #[test]
    fn test_get_branch_commits_zero() {
        let temp = create_test_repo();
        let base = get_current_branch_name(temp.path());

        run_git(temp.path(), &["checkout", "-b", "feature-nocommits"]);

        let commits = get_branch_commits(temp.path(), "feature-nocommits", &base, 0, 20).unwrap();
        assert!(commits.is_empty());
    }

    // T-DIFF-041: No upstream, fallback to main
    #[test]
    fn test_detect_base_branch_no_upstream() {
        let temp = create_test_repo();
        run_git(temp.path(), &["checkout", "-b", "feature-noup"]);

        let base = detect_base_branch(temp.path(), "feature-noup").unwrap();
        assert_eq!(base, "main");
    }

    // T-DIFF-042: Base branch candidates
    #[test]
    fn test_list_base_branch_candidates() {
        let temp = create_test_repo();
        let default_branch = get_current_branch_name(temp.path());

        // Create develop branch
        run_git(temp.path(), &["branch", "develop"]);

        let candidates = list_base_branch_candidates(temp.path()).unwrap();
        assert!(candidates.contains(&default_branch));
        assert!(candidates.contains(&"develop".to_string()));
    }

    // T-DIFF-050: Summary aggregation
    #[test]
    fn test_get_git_change_summary() {
        let temp = create_test_repo();
        let base = create_repo_with_feature(&temp);

        let summary = get_git_change_summary(temp.path(), "feature", &base).unwrap();
        assert_eq!(summary.file_count, 3);
        assert_eq!(summary.commit_count, 3);
        assert_eq!(summary.stash_count, 0);
        assert_eq!(summary.base_branch, base);
    }
}
