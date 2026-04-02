//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Built with the Elm Architecture (Model / View / Update) pattern.
#![allow(dead_code)]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    let log_config = gwt_core::logging::LogConfig::default();
    let _profiling_guard = gwt_core::logging::init_logger(&log_config).ok();

    let cwd = std::env::current_dir().unwrap_or_default();
    let repo_root = resolve_repo_root(&cwd);

    // Note: Skill registration (FR-073) is deferred to agent launch time,
    // not at gwt-tui startup. Startup should avoid mutating project-local
    // managed assets under .gwt while the binary is running from source.

    gwt_tui::app::run(repo_root)
}

/// Resolve the effective repository root from a given directory.
///
/// - If `cwd` is already a git repo (Normal / Worktree), return it as-is.
/// - Falls back to `cwd` when no repository can be detected (NonRepo / Empty).
fn resolve_repo_root(cwd: &std::path::Path) -> std::path::PathBuf {
    use gwt_core::git::{detect_repo_type, RepoType};

    match detect_repo_type(cwd) {
        RepoType::Normal | RepoType::Worktree => cwd.to_path_buf(),
        RepoType::NonRepo | RepoType::Empty => cwd.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_repo_root_returns_cwd_for_normal_repo() {
        let temp = tempfile::TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert_eq!(resolve_repo_root(temp.path()), temp.path());
    }

    #[test]
    fn resolve_repo_root_falls_back_to_cwd_when_no_repo() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("dummy"), "x").unwrap();
        assert_eq!(resolve_repo_root(temp.path()), temp.path().to_path_buf());
    }

    #[test]
    fn resolve_repo_root_returns_cwd_for_empty_dir() {
        let temp = tempfile::TempDir::new().unwrap();
        assert_eq!(resolve_repo_root(temp.path()), temp.path().to_path_buf());
    }
}
