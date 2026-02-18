//! Recent projects persistence (`~/.gwt/recent-projects.toml`)
//!
//! Stores project open history as TOML.
//! - Entries are deduplicated by path (same path updates `last_opened`).
//! - Non-existent paths are automatically removed on load.

use crate::config::migration::write_atomic;
use crate::error::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tracing::{debug, warn};

const RECENT_PROJECTS_FILE: &str = "recent-projects.toml";

/// A single recent project entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecentProject {
    pub path: String,
    pub last_opened: DateTime<Utc>,
}

/// Top-level TOML structure for `recent-projects.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct RecentProjectsData {
    #[serde(default)]
    projects: Vec<RecentProject>,
}

fn recent_projects_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".gwt").join(RECENT_PROJECTS_FILE))
}

/// Load recent projects from `~/.gwt/recent-projects.toml`.
///
/// Returns entries sorted by `last_opened` descending (most recent first).
/// Non-existent paths are automatically removed and the file is rewritten.
pub fn load_recent_projects() -> Vec<RecentProject> {
    let Some(path) = recent_projects_path() else {
        return vec![];
    };

    if !path.exists() {
        return vec![];
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            warn!(
                category = "config",
                path = %path.display(),
                error = %e,
                "Failed to read recent projects file"
            );
            return vec![];
        }
    };

    let data: RecentProjectsData = match toml::from_str(&content) {
        Ok(d) => d,
        Err(e) => {
            warn!(
                category = "config",
                path = %path.display(),
                error = %e,
                "Failed to parse recent projects file"
            );
            return vec![];
        }
    };

    let before_count = data.projects.len();
    let mut projects: Vec<RecentProject> = data
        .projects
        .into_iter()
        .filter(|p| std::path::Path::new(&p.path).exists())
        .collect();

    // Sort by last_opened descending.
    projects.sort_by(|a, b| b.last_opened.cmp(&a.last_opened));

    // Rewrite file if stale entries were removed.
    if projects.len() != before_count {
        debug!(
            category = "config",
            removed = before_count - projects.len(),
            "Cleaned stale recent project entries"
        );
        let _ = save_recent_projects(&projects);
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
    let Some(path) = recent_projects_path() else {
        return vec![];
    };

    if !path.exists() {
        return vec![];
    }

    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };

    let data: RecentProjectsData = match toml::from_str(&content) {
        Ok(d) => d,
        Err(_) => return vec![],
    };

    data.projects
}

fn save_recent_projects(projects: &[RecentProject]) -> Result<()> {
    let Some(path) = recent_projects_path() else {
        return Ok(());
    };

    let data = RecentProjectsData {
        projects: projects.to_vec(),
    };
    let content =
        toml::to_string_pretty(&data).map_err(|e| crate::error::GwtError::ConfigWriteError {
            reason: format!("Failed to serialize recent projects: {}", e),
        })?;

    write_atomic(&path, &content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{TestEnvGuard, HOME_LOCK};
    use tempfile::TempDir;

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
