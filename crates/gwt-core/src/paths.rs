//! Utility functions for gwt filesystem paths.

use std::path::{Path, PathBuf};

use crate::{
    error::Result,
    repo_hash::{compute_path_hash, detect_repo_hash, RepoHash},
};

/// Return the gwt home directory (`~/.gwt/`).
pub fn gwt_home() -> PathBuf {
    dirs::home_dir()
        .expect("home directory must be resolvable")
        .join(".gwt")
}

/// Return the path to the global config file (`~/.gwt/config.toml`).
pub fn gwt_config_path() -> PathBuf {
    gwt_home().join("config.toml")
}

/// Return the sessions directory (`~/.gwt/sessions/`).
pub fn gwt_sessions_dir() -> PathBuf {
    gwt_home().join("sessions")
}

/// Return the cache directory (`~/.gwt/cache/`).
pub fn gwt_cache_dir() -> PathBuf {
    gwt_home().join("cache")
}

/// Return the project data root (`~/.gwt/projects/`).
pub fn gwt_projects_dir() -> PathBuf {
    gwt_home().join("projects")
}

/// Return the project data directory for a repository hash.
pub fn gwt_project_dir(repo_hash: &RepoHash) -> PathBuf {
    gwt_projects_dir().join(repo_hash.as_str())
}

/// Return the project scope hash for a repository path.
pub fn project_scope_hash(repo_path: &Path) -> RepoHash {
    detect_repo_hash(repo_path).unwrap_or_else(|| compute_path_hash(repo_path))
}

/// Return the project data directory for a repository path.
pub fn gwt_project_dir_for_repo_path(repo_path: &Path) -> PathBuf {
    let repo_hash = project_scope_hash(repo_path);
    gwt_project_dir(&repo_hash)
}

/// Return the global session state path (`~/.gwt/session.json`).
pub fn gwt_session_state_path() -> PathBuf {
    gwt_home().join("session.json")
}

/// Return the legacy logs root (`~/.gwt/logs/`).
pub fn gwt_logs_dir() -> PathBuf {
    gwt_home().join("logs")
}

/// Return the legacy coordination root (`~/.gwt/coordination/`).
pub fn gwt_coordination_root() -> PathBuf {
    gwt_home().join("coordination")
}

/// Return the coordination directory for a repository hash.
pub fn gwt_coordination_dir(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("coordination")
}

/// Return the coordination directory for a repository path, if `origin` exists.
pub fn gwt_coordination_dir_for_repo_path(repo_path: &Path) -> Option<PathBuf> {
    detect_repo_hash(repo_path).map(|repo_hash| gwt_coordination_dir(&repo_hash))
}

/// Return the structured-log directory for a repository hash.
pub fn gwt_project_logs_dir(repo_hash: &RepoHash) -> PathBuf {
    gwt_project_dir(repo_hash).join("logs")
}

/// Return the structured-log directory for a repository path, if `origin` exists.
pub fn gwt_project_logs_dir_for_repo_path(repo_path: &Path) -> Option<PathBuf> {
    detect_repo_hash(repo_path).map(|repo_hash| gwt_project_logs_dir(&repo_hash))
}

/// Return the shared runtime directory (`~/.gwt/runtime/`).
pub fn gwt_runtime_dir() -> PathBuf {
    gwt_runtime_dir_from(&gwt_home())
}

/// Return the project index runner path under the shared runtime directory.
pub fn gwt_runtime_runner_path() -> PathBuf {
    gwt_runtime_runner_path_from(&gwt_home())
}

/// Return the managed project-index virtualenv directory.
pub fn gwt_project_index_venv_dir() -> PathBuf {
    gwt_project_index_venv_dir_from(&gwt_home())
}

pub(crate) fn gwt_runtime_dir_from(gwt_home: &Path) -> PathBuf {
    gwt_home.join("runtime")
}

pub(crate) fn gwt_runtime_runner_path_from(gwt_home: &Path) -> PathBuf {
    gwt_runtime_dir_from(gwt_home).join("chroma_index_runner.py")
}

pub(crate) fn gwt_project_index_venv_dir_from(gwt_home: &Path) -> PathBuf {
    gwt_runtime_dir_from(gwt_home).join("chroma-venv")
}

/// Ensure that the directory at `path` exists, creating it recursively if
/// necessary.
pub fn ensure_dir(path: &Path) -> Result<()> {
    std::fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repo_hash::compute_repo_hash;

    #[test]
    fn gwt_home_ends_with_dot_gwt() {
        let home = gwt_home();
        assert!(home.ends_with(".gwt"));
    }

    #[test]
    fn gwt_config_path_ends_with_config_toml() {
        let p = gwt_config_path();
        assert_eq!(p.file_name().unwrap(), "config.toml");
        assert!(p.starts_with(gwt_home()));
    }

    #[test]
    fn gwt_sessions_dir_is_under_home() {
        let p = gwt_sessions_dir();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("sessions"));
    }

    #[test]
    fn gwt_cache_dir_is_under_home() {
        let p = gwt_cache_dir();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("cache"));
    }

    #[test]
    fn gwt_projects_dir_is_under_home() {
        let p = gwt_projects_dir();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("projects"));
    }

    #[test]
    fn gwt_project_dir_scopes_by_repo_hash() {
        let repo_hash = compute_repo_hash("https://github.com/example/project.git");
        let p = gwt_project_dir(&repo_hash);
        assert!(p.starts_with(gwt_projects_dir()));
        assert!(p.ends_with(format!("projects/{}", repo_hash.as_str())));
    }

    #[test]
    fn gwt_session_state_path_is_under_home() {
        let p = gwt_session_state_path();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("session.json"));
    }

    #[test]
    fn project_scope_hash_falls_back_for_non_repo_paths() {
        let tmp = tempfile::tempdir().unwrap();
        let hash = project_scope_hash(tmp.path());
        assert_eq!(hash.as_str().len(), 16);
        assert!(hash
            .as_str()
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn gwt_logs_dir_is_under_home() {
        let p = gwt_logs_dir();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("logs"));
    }

    #[test]
    fn gwt_project_logs_dir_scopes_by_repo_hash() {
        let repo_hash = compute_repo_hash("https://github.com/example/project.git");
        let p = gwt_project_logs_dir(&repo_hash);
        assert!(p.starts_with(gwt_project_dir(&repo_hash)));
        assert!(p.ends_with(format!("projects/{}/logs", repo_hash.as_str())));
    }

    #[test]
    fn gwt_coordination_dir_scopes_by_repo_hash() {
        let repo_hash = compute_repo_hash("https://github.com/example/project.git");
        let p = gwt_coordination_dir(&repo_hash);
        assert!(p.starts_with(gwt_project_dir(&repo_hash)));
        assert!(p.ends_with(format!("projects/{}/coordination", repo_hash.as_str())));
    }

    #[test]
    fn gwt_runtime_dir_is_under_home() {
        let p = gwt_runtime_dir();
        assert!(p.starts_with(gwt_home()));
        assert!(p.ends_with("runtime"));
    }

    #[test]
    fn gwt_runtime_runner_path_is_under_runtime_dir() {
        let p = gwt_runtime_runner_path();
        assert!(p.starts_with(gwt_runtime_dir()));
        assert_eq!(p.file_name().unwrap(), "chroma_index_runner.py");
    }

    #[test]
    fn gwt_project_index_venv_dir_is_under_runtime_dir() {
        let p = gwt_project_index_venv_dir();
        assert!(p.starts_with(gwt_runtime_dir()));
        assert_eq!(p.file_name().unwrap(), "chroma-venv");
    }

    #[test]
    fn ensure_dir_creates_missing_directory() {
        let tmp = std::env::temp_dir().join("gwt_test_ensure_dir");
        let _ = std::fs::remove_dir_all(&tmp);

        let target = tmp.join("a").join("b").join("c");
        assert!(!target.exists());
        ensure_dir(&target).unwrap();
        assert!(target.is_dir());

        // Calling again on existing dir is a no-op.
        ensure_dir(&target).unwrap();

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn ensure_dir_succeeds_for_existing_directory() {
        let tmp = std::env::temp_dir();
        ensure_dir(&tmp).unwrap();
    }
}
