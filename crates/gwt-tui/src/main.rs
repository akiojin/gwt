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
/// - If `cwd` is already a git repo (Normal / Worktree / Bare), return it as-is.
/// - If `cwd` is NonRepo or Empty, look for a `*.git` bare repo inside it
///   via [`gwt_core::git::find_bare_repo_in_dir`] and return it if found.
/// - Falls back to `cwd` when no repository can be detected.
fn resolve_repo_root(cwd: &std::path::Path) -> std::path::PathBuf {
    use gwt_core::git::{detect_repo_type, find_bare_repo_in_dir, RepoType};

    match detect_repo_type(cwd) {
        RepoType::Normal | RepoType::Worktree | RepoType::Bare => cwd.to_path_buf(),
        RepoType::NonRepo | RepoType::Empty => {
            find_bare_repo_in_dir(cwd).unwrap_or_else(|| cwd.to_path_buf())
        }
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
    fn resolve_repo_root_finds_bare_repo_in_parent() {
        let temp = tempfile::TempDir::new().unwrap();
        let bare = temp.path().join("project.git");
        std::process::Command::new("git")
            .args(["init", "--bare", bare.to_str().unwrap()])
            .output()
            .unwrap();
        assert_eq!(resolve_repo_root(temp.path()), bare);
    }

    #[test]
    fn resolve_repo_root_falls_back_to_cwd_when_no_repo() {
        let temp = tempfile::TempDir::new().unwrap();
        std::fs::write(temp.path().join("dummy"), "x").unwrap();
        assert_eq!(resolve_repo_root(temp.path()), temp.path().to_path_buf());
    }

    #[test]
    fn resolve_repo_root_returns_bare_directly() {
        let temp = tempfile::TempDir::new().unwrap();
        std::process::Command::new("git")
            .args(["init", "--bare"])
            .current_dir(temp.path())
            .output()
            .unwrap();
        assert_eq!(resolve_repo_root(temp.path()), temp.path());
    }
}
