//! Git diff helpers

use std::path::{Path, PathBuf};

use gwt_core::{GwtError, Result};
use serde::{Deserialize, Serialize};

/// File status in the working tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileStatus {
    Staged,
    Unstaged,
    Untracked,
}

impl std::fmt::Display for FileStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Staged => write!(f, "staged"),
            Self::Unstaged => write!(f, "unstaged"),
            Self::Untracked => write!(f, "untracked"),
        }
    }
}

/// A single file entry from `git status --porcelain`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    /// Path relative to the repository root.
    pub path: PathBuf,
    /// Whether the file is staged, unstaged, or untracked.
    pub status: FileStatus,
}

impl FileEntry {
    /// Lazily load the diff content for this file.
    ///
    /// Returns the diff output for staged or unstaged files, or the file
    /// contents for untracked files.
    pub fn diff_content(&self, repo_path: &Path) -> Result<String> {
        match self.status {
            FileStatus::Staged => {
                let output = gwt_core::process::git_command()
                    .args(["diff", "--cached", "--", self.path.to_str().unwrap_or("")])
                    .current_dir(repo_path)
                    .output()
                    .map_err(|e| GwtError::GitOperationFailed {
                        operation: "diff --cached".into(),
                        details: e.to_string(),
                    })?;
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            }
            FileStatus::Unstaged => {
                let output = gwt_core::process::git_command()
                    .args(["diff", "--", self.path.to_str().unwrap_or("")])
                    .current_dir(repo_path)
                    .output()
                    .map_err(|e| GwtError::GitOperationFailed {
                        operation: "diff".into(),
                        details: e.to_string(),
                    })?;
                Ok(String::from_utf8_lossy(&output.stdout).to_string())
            }
            FileStatus::Untracked => {
                let full = repo_path.join(&self.path);
                std::fs::read_to_string(&full).map_err(GwtError::Io)
            }
        }
    }
}

/// Get the working tree status as a list of `FileEntry`.
pub fn get_status(repo_path: &Path) -> Result<Vec<FileEntry>> {
    let output = gwt_core::process::git_command()
        .args(["status", "--porcelain=v1"])
        .current_dir(repo_path)
        .output()
        .map_err(|e| GwtError::GitOperationFailed {
            operation: "status".into(),
            details: e.to_string(),
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        return Err(GwtError::GitOperationFailed {
            operation: "status".into(),
            details: stderr,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_porcelain_status(&stdout))
}

/// Parse `git status --porcelain=v1` output.
pub fn parse_porcelain_status(output: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();

    for line in output.lines() {
        if line.len() < 3 {
            continue;
        }

        let index = line.as_bytes()[0];
        let worktree = line.as_bytes()[1];
        let path = PathBuf::from(line[3..].trim_matches('"'));

        // Untracked
        if index == b'?' && worktree == b'?' {
            entries.push(FileEntry {
                path,
                status: FileStatus::Untracked,
            });
            continue;
        }

        // Staged changes (index has a letter, worktree is space or matching)
        if index != b' ' && index != b'?' {
            entries.push(FileEntry {
                path: path.clone(),
                status: FileStatus::Staged,
            });
        }

        // Unstaged changes (worktree has a modification marker)
        if worktree != b' ' && worktree != b'?' {
            entries.push(FileEntry {
                path,
                status: FileStatus::Unstaged,
            });
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_untracked() {
        let entries = parse_porcelain_status("?? newfile.txt\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, FileStatus::Untracked);
        assert_eq!(entries[0].path, PathBuf::from("newfile.txt"));
    }

    #[test]
    fn parse_staged() {
        let entries = parse_porcelain_status("M  src/lib.rs\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, FileStatus::Staged);
    }

    #[test]
    fn parse_unstaged() {
        let entries = parse_porcelain_status(" M src/lib.rs\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, FileStatus::Unstaged);
    }

    #[test]
    fn parse_both_staged_and_unstaged() {
        let entries = parse_porcelain_status("MM src/lib.rs\n");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].status, FileStatus::Staged);
        assert_eq!(entries[1].status, FileStatus::Unstaged);
    }

    #[test]
    fn parse_added() {
        let entries = parse_porcelain_status("A  new.rs\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, FileStatus::Staged);
    }

    #[test]
    fn parse_deleted() {
        let entries = parse_porcelain_status("D  old.rs\n");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, FileStatus::Staged);
    }

    #[test]
    fn parse_mixed_status() {
        let output = "M  staged.rs\n M unstaged.rs\n?? untracked.txt\nA  added.rs\n";
        let entries = parse_porcelain_status(output);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[0].status, FileStatus::Staged);
        assert_eq!(entries[1].status, FileStatus::Unstaged);
        assert_eq!(entries[2].status, FileStatus::Untracked);
        assert_eq!(entries[3].status, FileStatus::Staged);
    }

    #[test]
    fn parse_empty() {
        let entries = parse_porcelain_status("");
        assert!(entries.is_empty());
    }

    #[test]
    fn file_status_display() {
        assert_eq!(FileStatus::Staged.to_string(), "staged");
        assert_eq!(FileStatus::Unstaged.to_string(), "unstaged");
        assert_eq!(FileStatus::Untracked.to_string(), "untracked");
    }

    #[test]
    fn get_status_in_clean_repo() {
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

        let entries = get_status(path).unwrap();
        assert!(entries.is_empty());
    }

    #[test]
    fn get_status_with_untracked_file() {
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
        std::fs::write(path.join("new.txt"), "hello").unwrap();

        let entries = get_status(path).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].status, FileStatus::Untracked);
    }
}
