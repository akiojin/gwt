//! Recent projects persistence inside `~/.gwt/config.toml`.
//!
//! - Entries are deduplicated by path (same path updates `last_opened`).
//! - Non-existent paths are automatically removed on load.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use super::settings::Settings;
use crate::error::Result;

/// A single recent project entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentProject {
    pub path: String,
    pub last_opened: DateTime<Utc>,
}

/// Top-level TOML structure for `[recent_projects]`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RecentProjectsConfig {
    #[serde(default)]
    pub projects: Vec<RecentProject>,
}

/// Load recent projects from `~/.gwt/config.toml`.
///
/// Returns entries sorted by `last_opened` descending (most recent first).
pub fn load_recent_projects() -> Vec<RecentProject> {
    let settings = match Settings::load_global_raw() {
        Ok(settings) => settings,
        Err(e) => {
            warn!(
                category = "config",
                error = %e,
                "Failed to load config.toml for recent projects"
            );
            return vec![];
        }
    };

    let before_count = settings.recent_projects.projects.len();
    let mut projects: Vec<RecentProject> = settings
        .recent_projects
        .projects
        .into_iter()
        .filter(|p| std::path::Path::new(&p.path).exists())
        .collect();

    projects.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));

    if projects.len() != before_count {
        debug!(
            category = "config",
            removed = before_count - projects.len(),
            "Cleaned stale recent project entries"
        );
    }

    projects
}

/// Record a project path in the recent projects history.
///
/// If the path already exists, its `last_opened` is updated.
/// Otherwise a new entry is appended.
pub fn record_recent_project(path: &str) -> Result<()> {
    let mut projects = load_recent_projects_raw();
    let now = Utc::now();

    if let Some(existing) = projects.iter_mut().find(|p| p.path == path) {
        existing.last_opened = now;
    } else {
        projects.push(RecentProject {
            path: path.to_string(),
            last_opened: now,
        });
    }

    save_recent_projects(&projects)
}

/// Load raw entries without filtering non-existent paths.
fn load_recent_projects_raw() -> Vec<RecentProject> {
    Settings::load_global_raw()
        .map(|settings| settings.recent_projects.projects)
        .unwrap_or_default()
}

fn save_recent_projects(projects: &[RecentProject]) -> Result<()> {
    Settings::update_global(|settings| {
        settings.recent_projects = RecentProjectsConfig {
            projects: projects.to_vec(),
        };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::config::{TestEnvGuard, HOME_LOCK};

    #[test]
    fn test_record_and_load() {
        let _lock = HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _guard = TestEnvGuard::new(temp.path());

        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        // Create a directory to use as a project path.
        let project_dir = temp.path().join("my-project");
        std::fs::create_dir_all(&project_dir).unwrap();

        let project_path = project_dir.to_string_lossy().to_string();
        record_recent_project(&project_path).unwrap();

        let projects = load_recent_projects();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].path, project_path);
    }

    #[test]
    fn test_duplicate_updates_last_opened() {
        let _lock = HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _guard = TestEnvGuard::new(temp.path());

        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        let project_dir = temp.path().join("my-project");
        std::fs::create_dir_all(&project_dir).unwrap();
        let project_path = project_dir.to_string_lossy().to_string();

        record_recent_project(&project_path).unwrap();
        let first_time = load_recent_projects()[0].last_opened;

        // Small delay to get a different timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));
        record_recent_project(&project_path).unwrap();
        let second_time = load_recent_projects()[0].last_opened;

        assert!(second_time >= first_time);
        // Still only one entry
        assert_eq!(load_recent_projects().len(), 1);
    }

    #[test]
    fn record_recent_project_does_not_persist_env_overrides() {
        let _lock = HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _guard = TestEnvGuard::new(temp.path());

        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        let project_dir = temp.path().join("my-project");
        std::fs::create_dir_all(&project_dir).unwrap();

        std::env::set_var("GWT_DOCKER_FORCE_HOST", "1");
        record_recent_project(&project_dir.to_string_lossy()).unwrap();
        std::env::remove_var("GWT_DOCKER_FORCE_HOST");

        let raw = Settings::load_global_raw().unwrap();
        assert!(!raw.docker.force_host);
        assert_eq!(raw.recent_projects.projects.len(), 1);
    }

    #[test]
    fn test_stale_entries_removed() {
        let _lock = HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _guard = TestEnvGuard::new(temp.path());

        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        // Record a path that does not exist.
        let fake_path = temp
            .path()
            .join("nonexistent")
            .to_string_lossy()
            .to_string();
        record_recent_project(&fake_path).unwrap();

        // load_recent_projects should filter it out.
        let projects = load_recent_projects();
        assert!(projects.is_empty());
    }

    #[test]
    fn test_sorted_by_last_opened_desc() {
        let _lock = HOME_LOCK.lock().unwrap();
        let temp = TempDir::new().unwrap();
        let _guard = TestEnvGuard::new(temp.path());

        let gwt_dir = temp.path().join(".gwt");
        std::fs::create_dir_all(&gwt_dir).unwrap();

        let dir_a = temp.path().join("project-a");
        let dir_b = temp.path().join("project-b");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();

        record_recent_project(&dir_a.to_string_lossy()).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        record_recent_project(&dir_b.to_string_lossy()).unwrap();

        let projects = load_recent_projects();
        assert_eq!(projects.len(), 2);
        // Most recent first
        assert_eq!(projects[0].path, dir_b.to_string_lossy().to_string());
        assert_eq!(projects[1].path, dir_a.to_string_lossy().to_string());
    }
}
