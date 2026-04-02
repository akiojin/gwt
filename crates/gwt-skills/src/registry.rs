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
            let entry =
                entry.map_err(|e| RegistryError::Io(format!("{}: {e}", dir.display())))?;
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

/// Predefined builtin skill identifiers shipped with gwt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BuiltinSkill {
    GwtPr,
    GwtPrCheck,
    GwtPrFix,
    GwtSpecOps,
    GwtSpecRegister,
    GwtSpecImplement,
    GwtIssueRegister,
    GwtIssueResolve,
}

impl BuiltinSkill {
    /// Machine name for this builtin skill.
    pub fn name(self) -> &'static str {
        match self {
            Self::GwtPr => "gwt-pr",
            Self::GwtPrCheck => "gwt-pr-check",
            Self::GwtPrFix => "gwt-pr-fix",
            Self::GwtSpecOps => "gwt-spec-ops",
            Self::GwtSpecRegister => "gwt-spec-register",
            Self::GwtSpecImplement => "gwt-spec-implement",
            Self::GwtIssueRegister => "gwt-issue-register",
            Self::GwtIssueResolve => "gwt-issue-resolve",
        }
    }

    /// Short description of this builtin skill.
    pub fn description(self) -> &'static str {
        match self {
            Self::GwtPr => "Create or update GitHub Pull Requests",
            Self::GwtPrCheck => "Check GitHub PR status (CI, merge, review)",
            Self::GwtPrFix => "Fix CI failures, merge conflicts, and review comments",
            Self::GwtSpecOps => "Orchestrate SPEC lifecycle end-to-end",
            Self::GwtSpecRegister => "Create a new local SPEC directory",
            Self::GwtSpecImplement => "Implement SPEC tasks with test-first workflow",
            Self::GwtIssueRegister => "Register new GitHub work items",
            Self::GwtIssueResolve => "Resolve an existing GitHub Issue end-to-end",
        }
    }

    /// All builtin skill variants.
    pub fn all() -> &'static [BuiltinSkill] {
        &[
            Self::GwtPr,
            Self::GwtPrCheck,
            Self::GwtPrFix,
            Self::GwtSpecOps,
            Self::GwtSpecRegister,
            Self::GwtSpecImplement,
            Self::GwtIssueRegister,
            Self::GwtIssueResolve,
        ]
    }

    /// Convert to an `EmbeddedSkill` with a standard script path.
    pub fn to_embedded(self) -> EmbeddedSkill {
        EmbeddedSkill {
            name: self.name().to_string(),
            description: self.description().to_string(),
            script_path: PathBuf::from(format!(".claude/skills/{}/SKILL.md", self.name())),
            enabled: true,
        }
    }
}

/// Register all builtin skills into the given registry.
pub fn register_builtins(registry: &mut SkillRegistry) {
    for &builtin in BuiltinSkill::all() {
        registry.register(builtin.to_embedded());
    }
}

