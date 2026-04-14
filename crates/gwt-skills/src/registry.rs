//! Skill registry for embedded skill management.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A single embedded skill definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EmbeddedSkill {
    /// Human-readable skill name.
    pub name: String,
    /// Short description of what this skill does.
    pub description: String,
    /// Path to the skill script (relative to the skill directory).
    pub script_path: PathBuf,
    /// Whether this skill is currently enabled.
    pub enabled: bool,
}

/// Registry that holds and manages embedded skills.
#[derive(Debug, Default)]
pub struct SkillRegistry {
    skills: Vec<EmbeddedSkill>,
}

/// Result of updating a skill's enabled state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SkillUpdateResult {
    /// Whether a skill with the requested name existed.
    pub found: bool,
    /// Whether the enabled flag actually changed.
    pub changed: bool,
}

impl SkillRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a skill. Replaces any existing skill with the same name.
    pub fn register(&mut self, skill: EmbeddedSkill) {
        self.unregister(&skill.name);
        self.skills.push(skill);
    }

    /// Remove a skill by name. Returns `true` if a skill was removed.
    pub fn unregister(&mut self, name: &str) -> bool {
        let before = self.skills.len();
        self.skills.retain(|s| s.name != name);
        self.skills.len() < before
    }

    /// Update a skill's enabled state by name.
    pub fn set_enabled(&mut self, name: &str, enabled: bool) -> SkillUpdateResult {
        if let Some(skill) = self.skills.iter_mut().find(|skill| skill.name == name) {
            let changed = skill.enabled != enabled;
            skill.enabled = enabled;
            SkillUpdateResult {
                found: true,
                changed,
            }
        } else {
            SkillUpdateResult {
                found: false,
                changed: false,
            }
        }
    }

    /// List all registered skills.
    pub fn list(&self) -> &[EmbeddedSkill] {
        &self.skills
    }

    /// Scan a directory for skill definitions (JSON files named `skill.json`).
    pub fn load_from_dir(&mut self, dir: &Path) -> Result<usize, RegistryError> {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| RegistryError::Io(format!("{}: {e}", dir.display())))?;

        let mut count = 0;
        for entry in entries {
            let entry = entry.map_err(|e| RegistryError::Io(format!("{}: {e}", dir.display())))?;
            let skill_file = entry.path().join("skill.json");
            if skill_file.is_file() {
                let content = std::fs::read_to_string(&skill_file)
                    .map_err(|e| RegistryError::Io(format!("{}: {e}", skill_file.display())))?;
                let skill: EmbeddedSkill = serde_json::from_str(&content)
                    .map_err(|e| RegistryError::Parse(format!("{}: {e}", skill_file.display())))?;
                self.register(skill);
                count += 1;
            }
        }
        Ok(count)
    }
}

/// Errors from skill registry operations.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("IO error: {0}")]
    Io(String),

    #[error("Parse error: {0}")]
    Parse(String),
}
