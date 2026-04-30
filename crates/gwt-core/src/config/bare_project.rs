//! Schema and serialization helpers for `<project_root>/.gwt/project.toml`.
//!
//! Written by the Cleanup phase of a successful migration so subsequent gwt
//! launches can recognise the Nested Bare+Worktree layout without re-running
//! detection heuristics.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{GwtError, Result};

/// On-disk representation of a Nested Bare+Worktree project configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BareProjectConfig {
    /// Directory name of the bare repository (e.g. `gwt.git`).
    pub bare_repo_name: String,
    /// Origin URL captured at migration time, when available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
    /// RFC3339 timestamp of when the configuration was written.
    pub created_at: String,
    /// `"normal"` when produced by US-6 migration, `None` for fresh clones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub migrated_from: Option<String>,
}

impl BareProjectConfig {
    /// Path of the on-disk config file relative to a project root.
    pub fn config_path(project_root: &Path) -> PathBuf {
        project_root.join(".gwt").join("project.toml")
    }

    /// Read the config file, returning `Ok(None)` when it is absent.
    pub fn load(project_root: &Path) -> Result<Option<Self>> {
        let path = Self::config_path(project_root);
        if !path.exists() {
            return Ok(None);
        }
        let body = fs::read_to_string(&path).map_err(GwtError::Io)?;
        let parsed: Self = toml::from_str(&body)
            .map_err(|e| GwtError::Config(format!("parse {}: {e}", path.display())))?;
        Ok(Some(parsed))
    }

    /// Write the config file, creating the `.gwt/` directory if needed.
    pub fn save(&self, project_root: &Path) -> Result<()> {
        let path = Self::config_path(project_root);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(GwtError::Io)?;
        }
        let body = toml::to_string_pretty(self)
            .map_err(|e| GwtError::Config(format!("serialize project.toml: {e}")))?;
        fs::write(&path, body).map_err(GwtError::Io)?;
        Ok(())
    }
}
