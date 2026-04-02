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

/// PR status check states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CiStatus {
    Passing,
    Failing,
    Pending,
    Unknown,
}

/// Merge readiness states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeStatus {
    Ready,
    Blocked,
    Conflicts,
    Unknown,
}

/// Review states.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewStatus {
    Approved,
    ChangesRequested,
    Pending,
    Unknown,
}

/// Extended PR status report.
#[derive(Debug, Clone)]
pub struct PrCheckReport {
    pub ci: CiStatus,
    pub merge: MergeStatus,
    pub review: ReviewStatus,
    pub summary: String,
}

/// Generate an extended PR status report by inspecting the repository.
///
/// Runs `gh pr view` to gather CI, merge, and review states. Falls back
/// to `Unknown` states when `gh` is unavailable or the repo has no open PR.
pub fn gwt_pr_check_report(repo_path: &Path) -> Result<PrCheckReport, RegistryError> {
    let output = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            "--json",
            "statusCheckRollup,mergeable,reviewDecision,state,title",
        ])
        .current_dir(repo_path)
        .output()
        .map_err(|e| RegistryError::Io(format!("gh pr view: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Ok(PrCheckReport {
            ci: CiStatus::Unknown,
            merge: MergeStatus::Unknown,
            review: ReviewStatus::Unknown,
            summary: format!("No open PR or gh error: {}", stderr.trim()),
        });
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| RegistryError::Parse(format!("gh pr view JSON: {e}")))?;

    let ci = match json.get("statusCheckRollup") {
        Some(serde_json::Value::Array(checks)) => {
            let all_pass = checks.iter().all(|c| {
                c.get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == "SUCCESS" || s == "NEUTRAL" || s == "SKIPPED")
            });
            let any_fail = checks.iter().any(|c| {
                c.get("conclusion")
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| s == "FAILURE" || s == "CANCELLED" || s == "TIMED_OUT")
            });
            if checks.is_empty() {
                CiStatus::Pending
            } else if any_fail {
                CiStatus::Failing
            } else if all_pass {
                CiStatus::Passing
            } else {
                CiStatus::Pending
            }
        }
        _ => CiStatus::Unknown,
    };

    let merge = match json.get("mergeable").and_then(|v| v.as_str()) {
        Some("MERGEABLE") => MergeStatus::Ready,
        Some("CONFLICTING") => MergeStatus::Conflicts,
        Some(_) => MergeStatus::Blocked,
        None => MergeStatus::Unknown,
    };

    let review = match json.get("reviewDecision").and_then(|v| v.as_str()) {
        Some("APPROVED") => ReviewStatus::Approved,
        Some("CHANGES_REQUESTED") => ReviewStatus::ChangesRequested,
        Some("REVIEW_REQUIRED") => ReviewStatus::Pending,
        _ => ReviewStatus::Unknown,
    };

    let title = json
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("(untitled)");

    let summary = format!(
        "PR: {} | CI: {:?} | Merge: {:?} | Review: {:?}",
        title, ci, merge, review
    );

    Ok(PrCheckReport {
        ci,
        merge,
        review,
        summary,
    })
}
