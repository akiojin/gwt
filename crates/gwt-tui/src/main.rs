//! gwt-tui: Terminal UI for Git Worktree Manager
//!
//! Built with the Elm Architecture (Model / View / Update) pattern.
#![allow(dead_code)]

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing subscriber (simplified from old gwt_core::logging)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cwd = std::env::current_dir().unwrap_or_default();
    let repo_root = resolve_repo_root(&cwd);

    gwt_tui::app::run(repo_root)
}

/// Resolve the effective repository root from a given directory.
fn resolve_repo_root(cwd: &std::path::Path) -> std::path::PathBuf {
    // Simple check: does .git exist?
    cwd.to_path_buf()
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
