//! Repository scanner for deep context gathering
//!
//! Scans the repository to build context for LLM prompts:
//! CLAUDE.md, directory tree, build config, specs, source overview.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

/// Detected build system type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BuildSystem {
    Cargo,
    Npm,
    Unknown,
}

impl BuildSystem {
    /// Return the default test command for this build system
    pub fn test_command(&self) -> &'static str {
        match self {
            BuildSystem::Cargo => "cargo test",
            BuildSystem::Npm => "npm test",
            BuildSystem::Unknown => "",
        }
    }
}

/// Result of a repository scan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepositoryScanResult {
    /// Content of CLAUDE.md (if present)
    pub claude_md: Option<String>,
    /// Directory tree (top-level structure)
    pub directory_tree: String,
    /// Detected build system
    pub build_system: BuildSystem,
    /// Test command for the project
    pub test_command: String,
    /// Existing spec IDs found under specs/
    pub existing_specs: Vec<String>,
    /// Source module overview (key file names)
    pub source_overview: Vec<String>,
}

/// Scanner for gathering repository context
pub struct RepositoryScanner {
    repo_path: PathBuf,
}

impl RepositoryScanner {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Perform a full repository scan
    pub fn scan(&self) -> RepositoryScanResult {
        let claude_md = self.read_claude_md();
        let directory_tree = self.get_directory_tree();
        let build_system = self.detect_build_system();
        let test_command = build_system.test_command().to_string();
        let existing_specs = self.find_existing_specs();
        let source_overview = self.get_source_overview();

        RepositoryScanResult {
            claude_md,
            directory_tree,
            build_system,
            test_command,
            existing_specs,
            source_overview,
        }
    }

    fn read_claude_md(&self) -> Option<String> {
        let path = self.repo_path.join("CLAUDE.md");
        std::fs::read_to_string(path).ok()
    }

    fn get_directory_tree(&self) -> String {
        // Use git ls-tree for tracked files, fall back to simple listing
        let output = Command::new("git")
            .args(["ls-tree", "-r", "--name-only", "HEAD"])
            .current_dir(&self.repo_path)
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let files = String::from_utf8_lossy(&out.stdout);
                // Extract unique top-level directories and files
                let mut entries: Vec<String> = files
                    .lines()
                    .filter_map(|line| {
                        line.split('/').next().map(|s| s.to_string())
                    })
                    .collect();
                entries.sort();
                entries.dedup();
                entries.join("\n")
            }
            _ => {
                // Fallback: list directory entries
                Self::list_dir_entries(&self.repo_path)
            }
        }
    }

    fn list_dir_entries(path: &Path) -> String {
        match std::fs::read_dir(path) {
            Ok(entries) => {
                let mut names: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        !name.starts_with('.')
                    })
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect();
                names.sort();
                names.join("\n")
            }
            Err(_) => String::new(),
        }
    }

    fn detect_build_system(&self) -> BuildSystem {
        if self.repo_path.join("Cargo.toml").exists() {
            BuildSystem::Cargo
        } else if self.repo_path.join("package.json").exists() {
            BuildSystem::Npm
        } else {
            BuildSystem::Unknown
        }
    }

    fn find_existing_specs(&self) -> Vec<String> {
        let specs_dir = self.repo_path.join("specs");
        match std::fs::read_dir(&specs_dir) {
            Ok(entries) => {
                let mut specs: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with("SPEC-") {
                            Some(name)
                        } else {
                            None
                        }
                    })
                    .collect();
                specs.sort();
                specs
            }
            Err(_) => Vec::new(),
        }
    }

    fn get_source_overview(&self) -> Vec<String> {
        let output = Command::new("git")
            .args(["ls-tree", "-r", "--name-only", "HEAD"])
            .current_dir(&self.repo_path)
            .output();

        match output {
            Ok(out) if out.status.success() => {
                let files = String::from_utf8_lossy(&out.stdout);
                files
                    .lines()
                    .filter(|line| {
                        line.ends_with(".rs")
                            || line.ends_with(".ts")
                            || line.ends_with(".js")
                            || line.ends_with(".py")
                            || line.ends_with(".go")
                    })
                    .filter(|line| !line.contains("/target/") && !line.contains("/node_modules/"))
                    .map(|s| s.to_string())
                    .collect()
            }
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_system_test_command() {
        assert_eq!(BuildSystem::Cargo.test_command(), "cargo test");
        assert_eq!(BuildSystem::Npm.test_command(), "npm test");
        assert_eq!(BuildSystem::Unknown.test_command(), "");
    }

    #[test]
    fn test_scanner_new() {
        let scanner = RepositoryScanner::new("/tmp/test");
        assert_eq!(scanner.repo_path, PathBuf::from("/tmp/test"));
    }

    #[test]
    fn test_scan_nonexistent_repo() {
        let scanner = RepositoryScanner::new("/nonexistent/path/12345");
        let result = scanner.scan();
        assert!(result.claude_md.is_none());
        assert!(result.existing_specs.is_empty());
        assert_eq!(result.build_system, BuildSystem::Unknown);
    }

    #[test]
    fn test_list_dir_entries_nonexistent() {
        let result = RepositoryScanner::list_dir_entries(Path::new("/nonexistent/path/12345"));
        assert!(result.is_empty());
    }
}
