//! Skill/command/hook registration for agent integrations.
//!
//! Registration is project-scoped:
//! - Codex: `<project>/.codex/skills`
//! - Gemini: `<project>/.gemini/skills`
//! - Claude: `<project>/.claude/{skills,commands,hooks}` + `<project>/.claude/settings.local.json`

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tracing::{info, warn};

use super::Settings;
use crate::{error::GwtError, process::command};

// Auto-generated skill catalog from build.rs (parses plugins/gwt/skills/*/SKILL.md).
include!(concat!(env!("OUT_DIR"), "/skill_catalog_generated.rs"));

const MANAGED_SKILLS_BLOCK_BEGIN: &str = "<!-- BEGIN gwt managed skills -->";
const MANAGED_SKILLS_BLOCK_END: &str = "<!-- END gwt managed skills -->";

/// Generate the managed skills markdown block from the compiled skill catalog.
pub fn generate_managed_skills_block() -> String {
    // Category assignments (hardcoded since SKILL.md has no category field).
    const ISSUE_SPEC_SKILLS: &[&str] = &[
        "gwt-issue-register",
        "gwt-issue-resolve",
        "gwt-issue-search",
        "gwt-spec-register",
        "gwt-spec-clarify",
        "gwt-spec-plan",
        "gwt-spec-tasks",
        "gwt-spec-analyze",
        "gwt-spec-ops",
        "gwt-spec-implement",
    ];
    const PR_SKILLS: &[&str] = &["gwt-pr", "gwt-pr-check", "gwt-pr-fix"];
    // Everything else goes to Utilities.

    fn table_rows(names: &[&str]) -> String {
        let mut rows = String::new();
        for &name in names {
            let entry = SKILL_CATALOG.iter().find(|e| e.name == name);
            let (command, desc) = match entry {
                Some(e) => {
                    let cmd = if e.has_command {
                        format!("`/gwt:{}`", e.name)
                    } else {
                        "\u{2014}".to_string() // em-dash
                    };
                    (cmd, e.description)
                }
                None => continue,
            };
            rows.push_str(&format!("| {} | {} | {} |\n", name, command, desc));
        }
        rows
    }

    let utility_names: Vec<&str> = SKILL_CATALOG
        .iter()
        .map(|e| e.name)
        .filter(|n| !ISSUE_SPEC_SKILLS.contains(n) && !PR_SKILLS.contains(n))
        .collect();

    let mut block = String::new();
    block.push_str(MANAGED_SKILLS_BLOCK_BEGIN);
    block.push('\n');
    block.push_str("## Available Skills & Commands (gwt)\n\n");
    block.push_str("Skills are located in `.claude/skills/<name>/SKILL.md`.\n");
    block.push_str("Commands can be invoked as `/gwt:<command-name>`.\n\n");

    block.push_str("### Issue & SPEC Management\n\n");
    block.push_str("| Skill | Command | Description |\n");
    block.push_str("|-------|---------|-------------|\n");
    block.push_str(&table_rows(ISSUE_SPEC_SKILLS));
    block.push('\n');

    block.push_str("### PR Management\n\n");
    block.push_str("| Skill | Command | Description |\n");
    block.push_str("|-------|---------|-------------|\n");
    block.push_str(&table_rows(PR_SKILLS));
    block.push('\n');

    block.push_str("### Utilities\n\n");
    block.push_str("| Skill | Command | Description |\n");
    block.push_str("|-------|---------|-------------|\n");
    block.push_str(&table_rows(&utility_names));
    block.push('\n');

    block.push_str("### Recommended Workflow\n\n");
    block.push_str("See each skill's SKILL.md for detailed instructions:\n\n");
    block.push_str("1. **Register work** → `gwt-issue-register`\n");
    block.push_str("2. **Resolve an existing issue** → `gwt-issue-resolve`\n");
    block.push_str("3. **Create or select SPEC** → `gwt-spec-register` / `gwt-spec-ops`\n");
    block.push_str("4. **Clarify / plan / tasks / analyze** → `gwt-spec-ops`\n");
    block.push_str("5. **Implement SPEC tasks** → `gwt-spec-implement`\n");
    block.push_str("6. **Open PR** → `gwt-pr`\n");
    block.push_str("7. **Fix CI / reviews** → `gwt-pr-fix`\n");
    block.push_str(MANAGED_SKILLS_BLOCK_END);
    block.push('\n');

    block
}

/// Inject or replace the managed skills block in markdown content.
///
/// - If `<!-- BEGIN gwt managed skills -->` / `<!-- END gwt managed skills -->` markers
///   exist, replaces the block between them.
/// - If no markers exist, appends the block at the end (with blank-line separator).
/// - Returns an error for malformed marker pairs (e.g., BEGIN without END).
pub fn inject_managed_skills_block(content: &str) -> Result<String, String> {
    let has_begin = content.contains(MANAGED_SKILLS_BLOCK_BEGIN);
    let has_end = content.contains(MANAGED_SKILLS_BLOCK_END);

    if has_begin && !has_end {
        return Err(format!(
            "Malformed managed skills block: found BEGIN marker but missing END marker ('{}')",
            MANAGED_SKILLS_BLOCK_END
        ));
    }
    if !has_begin && has_end {
        return Err(format!(
            "Malformed managed skills block: found END marker but missing BEGIN marker ('{}')",
            MANAGED_SKILLS_BLOCK_BEGIN
        ));
    }

    let managed_block = generate_managed_skills_block();

    if has_begin && has_end {
        // Replace existing block
        let begin_idx = content.find(MANAGED_SKILLS_BLOCK_BEGIN).unwrap();
        let end_idx = content.find(MANAGED_SKILLS_BLOCK_END).unwrap();
        let after_end = end_idx + MANAGED_SKILLS_BLOCK_END.len();
        // Skip trailing newline after END marker if present
        let after_end = if content[after_end..].starts_with('\n') {
            after_end + 1
        } else {
            after_end
        };

        let mut result = String::new();
        result.push_str(&content[..begin_idx]);
        result.push_str(&managed_block);
        result.push_str(&content[after_end..]);
        Ok(result)
    } else {
        // Append to end
        let trimmed = content.trim_end();
        if trimmed.is_empty() {
            Ok(managed_block)
        } else {
            let mut result = String::new();
            result.push_str(trimmed);
            result.push_str("\n\n");
            result.push_str(&managed_block);
            Ok(result)
        }
    }
}

/// Managed file asset definition for project-local agent assets.
#[derive(Debug, Clone, Copy)]
struct ManagedAsset {
    relative_path: &'static str,
    body: &'static str,
    #[cfg_attr(not(unix), allow(dead_code))]
    executable: bool,
    rewrite_for_project: bool,
}

#[cfg(test)]
const MANAGED_SKILL_NAMES: &[&str] = &[
    "gwt-issue-register",
    "gwt-issue-resolve",
    "gwt-issue-search",
    "gwt-spec-register",
    "gwt-spec-clarify",
    "gwt-spec-plan",
    "gwt-spec-tasks",
    "gwt-spec-analyze",
    "gwt-spec-ops",
    "gwt-spec-implement",
    "gwt-pr-fix",
    "gwt-pr",
    "gwt-pr-check",
    "gwt-project-index",
    "gwt-pty-communication",
    "gwt-spec-to-issue-migration",
];

const PROJECT_SKILL_ASSETS: &[ManagedAsset] = &[
    ManagedAsset {
        relative_path: "skills/gwt-issue-register/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-issue-register/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-issue-resolve/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-issue-resolve/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-issue-resolve/scripts/inspect_issue.py",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-issue-resolve/scripts/inspect_issue.py"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-register/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-register/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-clarify/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-clarify/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-plan/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-plan/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-tasks/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-tasks/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-analyze/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-analyze/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-pr-fix/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-pr-fix/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-pr-fix/scripts/inspect_pr_checks.py",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-pr-fix/scripts/inspect_pr_checks.py"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-ops/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-ops/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-ops/scripts/spec_artifact.py",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-ops/scripts/spec_artifact.py"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-implement/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-implement/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-pr/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-pr/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-pr/references/pr-body-template.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-pr/references/pr-body-template.md"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "skills/gwt-pr-check/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-pr-check/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-pr-check/scripts/check_pr_status.py",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-pr-check/scripts/check_pr_status.py"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "skills/gwt-issue-search/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-issue-search/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-project-index/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-project-index/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-pty-communication/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-pty-communication/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-to-issue-migration/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-to-issue-migration/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-spec-to-issue-migration/scripts/migrate-specs-to-issues.mjs",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-to-issue-migration/scripts/migrate-specs-to-issues.mjs"
        )),
        executable: false,
        rewrite_for_project: false,
    },
];

const PROJECT_ROOT_ASSETS: &[ManagedAsset] = &[ManagedAsset {
    relative_path: "memory/constitution.md",
    body: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../memory/constitution.md"
    )),
    executable: false,
    rewrite_for_project: false,
}];

const LEGACY_MANAGED_GWT_HOOK_COMMANDS: &[&str] = &[
    "gwt hook UserPromptSubmit",
    "gwt hook PreToolUse",
    "gwt hook PostToolUse",
    "gwt hook Notification",
    "gwt hook Stop",
];

const LEGACY_MANAGED_HOOK_SCRIPT_BASENAMES: &[&str] = &[
    "forward-gwt-hook.sh",
    "block-git-branch-ops.sh",
    "block-cd-command.sh",
    "block-file-ops.sh",
    "block-git-dir-override.sh",
    "gwt-forward-hook.sh",
    "gwt-block-git-branch-ops.sh",
    "gwt-block-cd-command.sh",
    "gwt-block-file-ops.sh",
    "gwt-block-git-dir-override.sh",
];

const CLAUDE_COMMAND_ASSETS: &[ManagedAsset] = &[
    ManagedAsset {
        relative_path: "commands/gwt-issue-register.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-issue-register.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-issue-resolve.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-issue-resolve.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-spec-register.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-spec-register.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-spec-clarify.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-spec-clarify.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-spec-plan.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-spec-plan.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-spec-tasks.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-spec-tasks.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-spec-analyze.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-spec-analyze.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-spec-implement.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-spec-implement.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-pr-fix.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-pr-fix.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-pr-check.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-pr-check.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-pr.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-pr.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-issue-search.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-issue-search.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-project-index.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-project-index.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-pty-communication.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-pty-communication.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
];

const CLAUDE_HOOK_ASSETS: &[ManagedAsset] = &[
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-forward-hook.mjs",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-forward-hook.mjs"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-block-git-branch-ops.mjs",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-block-git-branch-ops.mjs"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-block-cd-command.mjs",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-block-cd-command.mjs"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-block-file-ops.mjs",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-block-file-ops.mjs"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-block-git-dir-override.mjs",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-block-git-dir-override.mjs"
        )),
        executable: false,
        rewrite_for_project: false,
    },
];

const LEGACY_MANAGED_ASSET_PATHS: &[&str] = &[
    "skills/gwt-fix-issue",
    "skills/gwt-fix-pr",
    "skills/gwt-issue-ops",
    "skills/gwt-issue-spec-ops",
    "commands/gwt-fix-issue.md",
    "commands/gwt-fix-pr.md",
    "commands/gwt-issue-ops.md",
    "commands/gwt-issue-spec-ops.md",
    "commands/gwt-spec-ops.md",
];

const SCOPE_NOT_CONFIGURED_CODE: &str = "SCOPE_NOT_CONFIGURED";
const SKILLS_PATH_UNAVAILABLE_CODE: &str = "SKILLS_PATH_UNAVAILABLE";
const SCOPE_NOT_CONFIGURED_MESSAGE: &str =
    "Skill registration is not configured. Enable it in Settings.";
const PROJECT_ROOT_REQUIRED_MESSAGE: &str =
    "Project root is required for project-scoped skill registration.";
const PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_BEGIN_MARKER: &str = "# BEGIN gwt managed local assets";
const PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_END_MARKER: &str = "# END gwt managed local assets";
const PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_LINES: &[&str] = &[
    "/.codex/skills/gwt-*/",
    "/.gemini/skills/gwt-*/",
    "/.claude/skills/gwt-*/",
    "/.claude/commands/gwt-*.md",
    "/.claude/hooks/scripts/gwt-*.mjs",
    "/.claude/settings.local.json",
    "/memory/constitution.md",
];
const LEGACY_PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_LINES: &[&str] = &[
    ".gwt/",
    "/.gwt/",
    ".codex/skills/gwt-*/",
    "/.codex/skills/gwt-*/**",
    ".gemini/skills/gwt-*/",
    "/.gemini/skills/gwt-*/**",
    ".claude/skills/gwt-*/",
    "/.claude/skills/gwt-*/**",
    ".claude/commands/gwt-*.md",
    ".claude/hooks/",
    ".claude/hooks/scripts/gwt-*.sh",
    "/.claude/hooks/scripts/gwt-*.sh",
    ".claude/settings.json",
    "/.claude/settings.json",
    "memory/constitution.md",
    "/memory/constitution.md",
];

/// Agent types that support skill registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillAgentType {
    Claude,
    Codex,
    Gemini,
}

impl SkillAgentType {
    pub fn all() -> &'static [SkillAgentType] {
        &[
            SkillAgentType::Claude,
            SkillAgentType::Codex,
            SkillAgentType::Gemini,
        ]
    }

    pub fn id(&self) -> &'static str {
        match self {
            SkillAgentType::Claude => "claude",
            SkillAgentType::Codex => "codex",
            SkillAgentType::Gemini => "gemini",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            SkillAgentType::Claude => "Claude Code",
            SkillAgentType::Codex => "Codex",
            SkillAgentType::Gemini => "Gemini",
        }
    }
}

/// Per-agent registration status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillAgentRegistrationStatus {
    pub agent_id: String,
    pub label: String,
    pub skills_path: Option<String>,
    pub registered: bool,
    pub missing_skills: Vec<String>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

/// Global registration status snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillRegistrationStatus {
    /// `ok` | `degraded` | `failed`
    pub overall: String,
    pub agents: Vec<SkillAgentRegistrationStatus>,
    /// Unix timestamp (milliseconds)
    pub last_checked_at: i64,
    pub last_error_message: Option<String>,
}

impl Default for SkillRegistrationStatus {
    fn default() -> Self {
        Self {
            overall: "failed".to_string(),
            agents: SkillAgentType::all()
                .iter()
                .map(|agent| SkillAgentRegistrationStatus {
                    agent_id: agent.id().to_string(),
                    label: agent.label().to_string(),
                    skills_path: None,
                    registered: false,
                    missing_skills: default_missing_items(*agent),
                    error_code: Some("NOT_CHECKED".to_string()),
                    error_message: Some(
                        "Skill registration status has not been checked yet.".to_string(),
                    ),
                })
                .collect(),
            last_checked_at: chrono::Utc::now().timestamp_millis(),
            last_error_message: Some(
                "Skill registration status has not been checked yet.".to_string(),
            ),
        }
    }
}

fn default_missing_items(agent: SkillAgentType) -> Vec<String> {
    let mut items = match agent {
        SkillAgentType::Claude => all_claude_assets()
            .map(|asset| format!(".claude/{}", asset.relative_path))
            .chain(std::iter::once(
                ".claude/settings.local.json hooks".to_string(),
            ))
            .collect(),
        SkillAgentType::Codex => project_asset_missing_items(".codex"),
        SkillAgentType::Gemini => project_asset_missing_items(".gemini"),
    };
    items.extend(
        PROJECT_ROOT_ASSETS
            .iter()
            .map(|asset| asset.relative_path.to_string()),
    );
    items
}

fn project_asset_missing_items(agent_root_name: &str) -> Vec<String> {
    PROJECT_SKILL_ASSETS
        .iter()
        .map(|asset| format!("{agent_root_name}/{}", asset.relative_path))
        .collect()
}

fn skill_registration_enabled(settings: &Settings) -> bool {
    settings
        .agent
        .skill_registration
        .as_ref()
        .map(|prefs| prefs.enabled)
        .unwrap_or(true)
}

fn skills_root_for(agent: SkillAgentType, project_root: Option<&Path>) -> Option<PathBuf> {
    let project_root = project_root?;
    match agent {
        SkillAgentType::Codex => Some(project_root.join(".codex").join("skills")),
        SkillAgentType::Gemini => Some(project_root.join(".gemini").join("skills")),
        SkillAgentType::Claude => None,
    }
}

fn agent_root_name(agent: SkillAgentType) -> &'static str {
    match agent {
        SkillAgentType::Claude => ".claude",
        SkillAgentType::Codex => ".codex",
        SkillAgentType::Gemini => ".gemini",
    }
}

fn agent_root_for(agent: SkillAgentType, project_root: Option<&Path>) -> Option<PathBuf> {
    let project_root = project_root?;
    Some(project_root.join(agent_root_name(agent)))
}

fn claude_root_for(project_root: Option<&Path>) -> Option<PathBuf> {
    project_root.map(|root| root.join(".claude"))
}

fn claude_settings_path_for(project_root: Option<&Path>) -> Option<PathBuf> {
    claude_root_for(project_root).map(|root| root.join("settings.local.json"))
}

#[cfg(test)]
fn register_agent_skills_at(root: &Path) -> Result<(), GwtError> {
    write_managed_assets(root, PROJECT_SKILL_ASSETS.iter(), ".codex")?;
    write_managed_assets(root, PROJECT_ROOT_ASSETS.iter(), ".")
}

fn register_claude_assets_at(project_root: &Path) -> Result<(), GwtError> {
    let root = project_root.join(".claude");
    cleanup_legacy_claude_hook_scripts(&root)?;

    // 3. Write file assets
    write_managed_assets(&root, all_claude_assets(), ".claude")?;

    // 4. Merge hooks into settings.local.json
    merge_managed_claude_hooks_into_settings(&root)
}

fn all_claude_assets() -> impl Iterator<Item = &'static ManagedAsset> {
    CLAUDE_COMMAND_ASSETS
        .iter()
        .chain(CLAUDE_HOOK_ASSETS.iter())
        .chain(PROJECT_SKILL_ASSETS.iter())
}

fn write_managed_assets<'a>(
    root: &Path,
    assets: impl Iterator<Item = &'a ManagedAsset>,
    root_name: &str,
) -> Result<(), GwtError> {
    std::fs::create_dir_all(root).map_err(|e| GwtError::ConfigWriteError {
        reason: format!(
            "Failed to create agent asset root {}: {}",
            root.display(),
            e
        ),
    })?;

    cleanup_legacy_managed_assets(root)?;

    for asset in assets {
        write_managed_asset(root, asset, root_name)?;
    }

    Ok(())
}

fn cleanup_legacy_managed_assets(root: &Path) -> Result<(), GwtError> {
    for relative_path in LEGACY_MANAGED_ASSET_PATHS {
        let path = root.join(relative_path);
        if path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| GwtError::ConfigWriteError {
                reason: format!(
                    "Failed to remove legacy managed asset {}: {}",
                    path.display(),
                    e
                ),
            })?;
        } else if path.is_file() {
            std::fs::remove_file(&path).map_err(|e| GwtError::ConfigWriteError {
                reason: format!(
                    "Failed to remove legacy managed asset {}: {}",
                    path.display(),
                    e
                ),
            })?;
        }
    }

    Ok(())
}

fn write_managed_asset(root: &Path, asset: &ManagedAsset, root_name: &str) -> Result<(), GwtError> {
    let path = root.join(asset.relative_path);
    let Some(parent) = path.parent() else {
        return Err(GwtError::ConfigWriteError {
            reason: format!("Invalid managed asset path: {}", path.display()),
        });
    };

    std::fs::create_dir_all(parent).map_err(|e| GwtError::ConfigWriteError {
        reason: format!(
            "Failed to create managed asset directory {}: {}",
            parent.display(),
            e
        ),
    })?;

    let content = if asset.rewrite_for_project {
        rewrite_project_asset_content(asset.body, root_name)
    } else {
        asset.body.to_string()
    };

    std::fs::write(&path, content).map_err(|e| GwtError::ConfigWriteError {
        reason: format!("Failed to write managed asset {}: {}", path.display(), e),
    })?;

    #[cfg(unix)]
    {
        if asset.executable {
            let metadata = std::fs::metadata(&path).map_err(|e| GwtError::ConfigWriteError {
                reason: format!("Failed to read metadata for {}: {}", path.display(), e),
            })?;
            let mut perms = metadata.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).map_err(|e| GwtError::ConfigWriteError {
                reason: format!(
                    "Failed to set executable permissions for {}: {}",
                    path.display(),
                    e
                ),
            })?;
        }
    }

    Ok(())
}

fn cleanup_legacy_claude_hook_scripts(root: &Path) -> Result<(), GwtError> {
    let hooks_json_path = root.join("hooks").join("hooks.json");
    if hooks_json_path.exists() {
        std::fs::remove_file(&hooks_json_path).map_err(|e| GwtError::ConfigWriteError {
            reason: format!(
                "Failed to remove legacy Claude hook template {}: {}",
                hooks_json_path.display(),
                e
            ),
        })?;
    }

    for basename in LEGACY_MANAGED_HOOK_SCRIPT_BASENAMES {
        let path = root.join("hooks").join("scripts").join(basename);
        if !path.exists() {
            continue;
        }
        std::fs::remove_file(&path).map_err(|e| GwtError::ConfigWriteError {
            reason: format!(
                "Failed to remove legacy Claude hook script {}: {}",
                path.display(),
                e
            ),
        })?;
    }

    Ok(())
}

fn git_path_for_project_root(project_root: &Path, git_path: &str) -> Result<PathBuf, GwtError> {
    let dot_git = project_root.join(".git");
    let output = match command("git")
        .arg("rev-parse")
        .arg("--git-path")
        .arg(git_path)
        .current_dir(project_root)
        .output()
    {
        Ok(output) => output,
        Err(_spawn_err) if dot_git.is_dir() => return Ok(dot_git.join(git_path)),
        Err(e) => {
            return Err(GwtError::ConfigWriteError {
                reason: format!(
                    "Failed to run git rev-parse --git-path {} in {}: {}",
                    git_path,
                    project_root.display(),
                    e
                ),
            });
        }
    };

    if output.status.success() {
        let resolved_raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if resolved_raw.is_empty() {
            return Err(GwtError::ConfigWriteError {
                reason: format!(
                    "git rev-parse --git-path {} returned an empty path for {}",
                    git_path,
                    project_root.display(),
                ),
            });
        }
        let resolved = PathBuf::from(&resolved_raw);
        return Ok(if resolved.is_absolute() {
            resolved
        } else {
            project_root.join(resolved)
        });
    }

    if dot_git.is_dir() {
        return Ok(dot_git.join(git_path));
    }

    Err(GwtError::ConfigWriteError {
        reason: format!(
            "Unable to resolve git path {} for {}: {}",
            git_path,
            project_root.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ),
    })
}

fn ensure_project_local_exclude_rules(project_root: &Path) -> Result<(), GwtError> {
    let exclude_path = git_path_for_project_root(project_root, "info/exclude")?;
    let existing = if exclude_path.exists() {
        std::fs::read_to_string(&exclude_path).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to read {}: {}", exclude_path.display(), e),
        })?
    } else {
        String::new()
    };

    let mut output_lines = Vec::new();
    let mut skipping_managed_block = false;

    for line in existing.lines() {
        if line == PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_BEGIN_MARKER {
            if skipping_managed_block {
                return Err(GwtError::ConfigWriteError {
                    reason: format!(
                        "Malformed managed exclude block in {}: nested begin marker",
                        exclude_path.display()
                    ),
                });
            }
            skipping_managed_block = true;
            continue;
        }
        if line == PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_END_MARKER {
            if !skipping_managed_block {
                return Err(GwtError::ConfigWriteError {
                    reason: format!(
                        "Malformed managed exclude block in {}: end marker without begin marker",
                        exclude_path.display()
                    ),
                });
            }
            skipping_managed_block = false;
            continue;
        }
        if skipping_managed_block {
            continue;
        }
        if PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_LINES.contains(&line)
            || LEGACY_PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_LINES.contains(&line)
        {
            continue;
        }
        output_lines.push(line.to_string());
    }

    if skipping_managed_block {
        return Err(GwtError::ConfigWriteError {
            reason: format!(
                "Malformed managed exclude block in {}: missing end marker",
                exclude_path.display()
            ),
        });
    }

    while output_lines.last().is_some_and(|line| line.is_empty()) {
        output_lines.pop();
    }
    if !output_lines.is_empty() {
        output_lines.push(String::new());
    }

    output_lines.push(PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_BEGIN_MARKER.to_string());
    for line in PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_LINES {
        output_lines.push((*line).to_string());
    }
    output_lines.push(PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_END_MARKER.to_string());

    let mut output = output_lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
    };

    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to create {}: {}", parent.display(), e),
        })?;
    }

    std::fs::write(&exclude_path, output).map_err(|e| GwtError::ConfigWriteError {
        reason: format!("Failed to write {}: {}", exclude_path.display(), e),
    })?;

    Ok(())
}

fn rewrite_project_asset_content(content: &str, root_name: &str) -> String {
    content
        .replace("${CLAUDE_PLUGIN_ROOT}", root_name)
        .replace("$CLAUDE_PLUGIN_ROOT", root_name)
        .replace("`skills/", &format!("`{root_name}/skills/"))
}

fn managed_hooks_definition() -> Value {
    serde_json::json!({
        "hooks": {
            "UserPromptSubmit": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": "node .claude/hooks/scripts/gwt-forward-hook.mjs UserPromptSubmit"
                }]
            }],
            "PreToolUse": [
                {
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": "node .claude/hooks/scripts/gwt-forward-hook.mjs PreToolUse"
                    }]
                },
                {
                    "matcher": "Bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": "node .claude/hooks/scripts/gwt-block-git-branch-ops.mjs"
                        },
                        {
                            "type": "command",
                            "command": "node .claude/hooks/scripts/gwt-block-cd-command.mjs"
                        },
                        {
                            "type": "command",
                            "command": "node .claude/hooks/scripts/gwt-block-file-ops.mjs"
                        },
                        {
                            "type": "command",
                            "command": "node .claude/hooks/scripts/gwt-block-git-dir-override.mjs"
                        }
                    ]
                }
            ],
            "PostToolUse": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": "node .claude/hooks/scripts/gwt-forward-hook.mjs PostToolUse"
                }]
            }],
            "Notification": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": "node .claude/hooks/scripts/gwt-forward-hook.mjs Notification"
                }]
            }],
            "Stop": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": "node .claude/hooks/scripts/gwt-forward-hook.mjs Stop"
                }]
            }]
        }
    })
}

fn merge_managed_claude_hooks_into_settings(claude_root: &Path) -> Result<(), GwtError> {
    let settings_path = claude_root.join("settings.local.json");

    if let Some(parent) = settings_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| GwtError::ConfigWriteError {
            reason: format!(
                "Failed to create Claude settings directory {}: {}",
                parent.display(),
                e
            ),
        })?;
    }

    let mut settings = if settings_path.exists() {
        let content =
            std::fs::read_to_string(&settings_path).map_err(|e| GwtError::ConfigWriteError {
                reason: format!(
                    "Failed to read Claude settings {}: {}",
                    settings_path.display(),
                    e
                ),
            })?;
        serde_json::from_str::<Value>(&content).map_err(|e| GwtError::ConfigParseError {
            reason: format!(
                "Failed to parse Claude settings {}: {}",
                settings_path.display(),
                e
            ),
        })?
    } else {
        serde_json::json!({})
    };

    if !settings.is_object() {
        settings = serde_json::json!({});
    }

    let hooks_definition = managed_hooks_definition();
    let Some(managed_hooks_map) = hooks_definition.get("hooks").and_then(|v| v.as_object()) else {
        return Err(GwtError::ConfigParseError {
            reason: "Managed Claude hooks template must have a hooks object".to_string(),
        });
    };
    let managed_hook_commands = managed_hook_commands_from_map(managed_hooks_map);

    let hooks_value = settings
        .as_object_mut()
        .expect("settings must be object")
        .entry("hooks".to_string())
        .or_insert_with(|| serde_json::json!({}));

    if !hooks_value.is_object() {
        *hooks_value = serde_json::json!({});
    }

    let hooks_map = hooks_value
        .as_object_mut()
        .expect("hooks must be object after normalization");

    // Remove stale managed entries before re-adding the latest definitions.
    for value in hooks_map.values_mut() {
        prune_managed_hook_entries(value, &managed_hook_commands);
    }

    for (event, managed_event_entries) in managed_hooks_map {
        let mut merged_entries: Vec<Value> = match hooks_map.get(event) {
            Some(existing) => existing
                .as_array()
                .cloned()
                .unwrap_or_else(|| vec![existing.clone()]),
            None => Vec::new(),
        };

        if let Some(entries) = managed_event_entries.as_array() {
            merged_entries.extend(entries.iter().cloned());
        } else {
            merged_entries.push(managed_event_entries.clone());
        }

        hooks_map.insert(event.clone(), Value::Array(merged_entries));
    }

    let output =
        serde_json::to_string_pretty(&settings).map_err(|e| GwtError::ConfigWriteError {
            reason: e.to_string(),
        })?;

    std::fs::write(&settings_path, output).map_err(|e| GwtError::ConfigWriteError {
        reason: format!(
            "Failed to write Claude settings {}: {}",
            settings_path.display(),
            e
        ),
    })?;

    Ok(())
}

fn prune_managed_hook_entries(value: &mut Value, managed_hook_commands: &[String]) {
    let Some(entries) = value.as_array_mut() else {
        if value
            .as_str()
            .map(|command| is_managed_hook_command(command, managed_hook_commands))
            .unwrap_or(false)
        {
            *value = Value::Array(vec![]);
        }
        return;
    };

    let mut retained_entries = Vec::new();

    for mut entry in entries.drain(..) {
        if let Some(entry_obj) = entry.as_object_mut() {
            if let Some(hooks) = entry_obj.get_mut("hooks").and_then(|v| v.as_array_mut()) {
                hooks.retain(|hook| {
                    let command = hook
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default();
                    !is_managed_hook_command(command, managed_hook_commands)
                });

                if hooks.is_empty() {
                    continue;
                }
            }

            retained_entries.push(entry);
            continue;
        }

        if let Some(command) = entry.as_str() {
            if is_managed_hook_command(command, managed_hook_commands) {
                continue;
            }
        }

        retained_entries.push(entry);
    }

    *entries = retained_entries;
}

fn is_managed_hook_command(command: &str, managed_hook_commands: &[String]) -> bool {
    managed_hook_commands
        .iter()
        .any(|managed_command| managed_command == command)
        || LEGACY_MANAGED_GWT_HOOK_COMMANDS.contains(&command)
        || command_script_basename(command)
            .map(|basename| LEGACY_MANAGED_HOOK_SCRIPT_BASENAMES.contains(&basename))
            .unwrap_or(false)
}

fn command_script_basename(command: &str) -> Option<&str> {
    let executable = command.split_whitespace().next().unwrap_or(command);
    Path::new(executable)
        .file_name()
        .and_then(|name| name.to_str())
}

fn managed_hook_commands_from_map(hooks_obj: &Map<String, Value>) -> Vec<String> {
    let mut commands = managed_hook_commands_with_events_from_map(hooks_obj)
        .into_iter()
        .map(|(_, command)| command)
        .collect::<Vec<_>>();
    commands.sort();
    commands.dedup();
    commands
}

fn managed_hook_commands_with_events_from_map(
    hooks_obj: &Map<String, Value>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();

    for (event, entries) in hooks_obj {
        let Some(entries_arr) = entries.as_array() else {
            continue;
        };

        for entry in entries_arr {
            let Some(hooks) = entry.get("hooks").and_then(|v| v.as_array()) else {
                continue;
            };
            for hook in hooks {
                if let Some(command) = hook.get("command").and_then(|v| v.as_str()) {
                    out.push((event.clone(), command.to_string()));
                }
            }
        }
    }

    out
}

fn required_managed_hook_commands() -> Vec<(String, String)> {
    let definition = managed_hooks_definition();

    let Some(hooks_obj) = definition.get("hooks").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

    managed_hook_commands_with_events_from_map(hooks_obj)
}

fn missing_managed_hook_events(settings_path: &Path) -> Vec<String> {
    let required = required_managed_hook_commands();
    if required.is_empty() {
        return vec!["hooks".to_string()];
    }

    let content = match std::fs::read_to_string(settings_path) {
        Ok(c) => c,
        Err(_) => {
            let mut events: Vec<String> = required.into_iter().map(|(event, _)| event).collect();
            events.sort();
            events.dedup();
            return events;
        }
    };

    let settings: Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => {
            let mut events: Vec<String> = required.into_iter().map(|(event, _)| event).collect();
            events.sort();
            events.dedup();
            return events;
        }
    };

    let hooks_obj = settings
        .get("hooks")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_else(Map::new);

    let mut missing = Vec::new();

    for (event, command) in required {
        let Some(event_value) = hooks_obj.get(&event) else {
            missing.push(event.clone());
            continue;
        };

        if !event_contains_hook_command(event_value, &command) {
            missing.push(event.clone());
        }
    }

    missing.sort();
    missing.dedup();
    missing
}

fn event_contains_hook_command(value: &Value, command: &str) -> bool {
    let Some(entries) = value.as_array() else {
        return false;
    };

    entries.iter().any(|entry| {
        entry
            .get("hooks")
            .and_then(|v| v.as_array())
            .map(|hooks| {
                hooks.iter().any(|hook| {
                    hook.get("command")
                        .and_then(|v| v.as_str())
                        .map(|c| c == command)
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
    })
}

/// Remove managed skill directories for one agent at the given root.
#[cfg(test)]
fn unregister_agent_skills_at(root: &Path) {
    for skill_name in MANAGED_SKILL_NAMES {
        let dir = root.join("skills").join(skill_name);
        if dir.exists() {
            if let Err(e) = std::fs::remove_dir_all(&dir) {
                warn!(
                    category = "skills",
                    path = %dir.display(),
                    error = %e,
                    "Failed to remove skill directory"
                );
            }
        }
    }
}

fn scope_unconfigured_status(agent: SkillAgentType) -> SkillAgentRegistrationStatus {
    SkillAgentRegistrationStatus {
        agent_id: agent.id().to_string(),
        label: agent.label().to_string(),
        skills_path: None,
        registered: false,
        missing_skills: default_missing_items(agent),
        error_code: Some(SCOPE_NOT_CONFIGURED_CODE.to_string()),
        error_message: Some(SCOPE_NOT_CONFIGURED_MESSAGE.to_string()),
    }
}

fn path_unavailable_status(
    agent: SkillAgentType,
    reason: &str,
    path_hint: Option<String>,
) -> SkillAgentRegistrationStatus {
    SkillAgentRegistrationStatus {
        agent_id: agent.id().to_string(),
        label: agent.label().to_string(),
        skills_path: path_hint,
        registered: false,
        missing_skills: default_missing_items(agent),
        error_code: Some(SKILLS_PATH_UNAVAILABLE_CODE.to_string()),
        error_message: Some(reason.to_string()),
    }
}

/// Register managed assets for one agent using an explicit project root.
pub fn register_agent_skills_with_settings_at_project_root(
    agent: SkillAgentType,
    settings: &Settings,
    project_root: Option<&Path>,
) -> Result<(), GwtError> {
    // Force registration is no longer needed with project-scoped assets.
    if !skill_registration_enabled(settings) {
        return Err(GwtError::ConfigWriteError {
            reason: SCOPE_NOT_CONFIGURED_MESSAGE.to_string(),
        });
    }

    let Some(project_root) = project_root else {
        return Err(GwtError::ConfigWriteError {
            reason: PROJECT_ROOT_REQUIRED_MESSAGE.to_string(),
        });
    };

    write_managed_assets(project_root, PROJECT_ROOT_ASSETS.iter(), ".")?;

    match agent {
        SkillAgentType::Claude => register_claude_assets_at(project_root),
        SkillAgentType::Codex | SkillAgentType::Gemini => {
            let Some(root) = agent_root_for(agent, Some(project_root)) else {
                return Err(GwtError::ConfigWriteError {
                    reason: format!("{} asset root could not be resolved.", agent.label()),
                });
            };
            write_managed_assets(&root, PROJECT_SKILL_ASSETS.iter(), agent_root_name(agent))
        }
    }
}

/// Register managed skills for all supported agents with explicit settings and project root.
pub fn register_all_skills_with_settings_at_project_root(
    settings: &Settings,
    project_root: Option<&Path>,
) -> Result<(), GwtError> {
    if let Some(project_root) = project_root {
        ensure_project_local_exclude_rules(project_root)?;
        write_managed_assets(project_root, PROJECT_ROOT_ASSETS.iter(), ".")?;
    }

    let mut failures = Vec::new();

    for agent in SkillAgentType::all() {
        let result =
            register_agent_skills_with_settings_at_project_root(*agent, settings, project_root);

        if let Err(err) = result {
            warn!(
                category = "skills",
                agent = agent.id(),
                error = %err,
                "Managed skill registration failed for one agent"
            );
            failures.push(format!("{}: {err}", agent.label()));
        }
    }

    if !failures.is_empty() {
        return Err(GwtError::ConfigWriteError {
            reason: format!(
                "Failed to register project-local managed assets for {} agent(s): {}",
                failures.len(),
                failures.join(" | ")
            ),
        });
    }

    Ok(())
}

fn status_for(
    agent: SkillAgentType,
    settings: &Settings,
    project_root: Option<&Path>,
) -> SkillAgentRegistrationStatus {
    if !skill_registration_enabled(settings) {
        return scope_unconfigured_status(agent);
    }

    let Some(project_root) = project_root else {
        return path_unavailable_status(agent, PROJECT_ROOT_REQUIRED_MESSAGE, None);
    };

    if agent == SkillAgentType::Claude {
        return status_for_claude(Some(project_root));
    }

    let root = agent_root_for(agent, Some(project_root));
    let skills_path = skills_root_for(agent, Some(project_root))
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    let Some(root) = root else {
        return path_unavailable_status(agent, "Skills path could not be resolved.", skills_path);
    };

    let mut missing = Vec::new();
    for asset in PROJECT_SKILL_ASSETS {
        let asset_path = root.join(asset.relative_path);
        if !asset_path.exists() {
            missing.push(format!(
                "{}/{}",
                agent_root_name(agent),
                asset.relative_path
            ));
        }
    }
    for asset in PROJECT_ROOT_ASSETS {
        let asset_path = project_root.join(asset.relative_path);
        if !asset_path.exists() {
            missing.push(asset.relative_path.to_string());
        }
    }

    let registered = missing.is_empty();
    SkillAgentRegistrationStatus {
        agent_id: agent.id().to_string(),
        label: agent.label().to_string(),
        skills_path,
        registered,
        missing_skills: missing.clone(),
        error_code: if registered {
            None
        } else {
            Some("PROJECT_ASSETS_MISSING".to_string())
        },
        error_message: if registered {
            None
        } else {
            Some(format!(
                "{} project assets are incomplete: {}",
                agent.label(),
                missing.join(", ")
            ))
        },
    }
}

fn status_for_claude(project_root: Option<&Path>) -> SkillAgentRegistrationStatus {
    let claude_root = claude_root_for(project_root);
    let skills_path = claude_root
        .as_ref()
        .map(|p| p.to_string_lossy().to_string());

    let Some(claude_root) = claude_root else {
        return path_unavailable_status(
            SkillAgentType::Claude,
            PROJECT_ROOT_REQUIRED_MESSAGE,
            skills_path,
        );
    };

    let settings_path = claude_settings_path_for(project_root)
        .unwrap_or_else(|| claude_root.join("settings.local.json"));

    let mut missing_items = Vec::new();

    for asset in all_claude_assets() {
        let asset_path = claude_root.join(asset.relative_path);
        if !asset_path.exists() {
            missing_items.push(format!(".claude/{}", asset.relative_path));
        }
    }
    if let Some(project_root) = project_root {
        for asset in PROJECT_ROOT_ASSETS {
            let asset_path = project_root.join(asset.relative_path);
            if !asset_path.exists() {
                missing_items.push(asset.relative_path.to_string());
            }
        }
    }

    if !settings_path.exists() {
        missing_items.push(".claude/settings.local.json".to_string());
    } else {
        for event in missing_managed_hook_events(&settings_path) {
            missing_items.push(format!(".claude/settings.local.json hooks.{event}"));
        }
    }

    let registered = missing_items.is_empty();

    SkillAgentRegistrationStatus {
        agent_id: SkillAgentType::Claude.id().to_string(),
        label: SkillAgentType::Claude.label().to_string(),
        skills_path,
        registered,
        missing_skills: missing_items.clone(),
        error_code: if registered {
            None
        } else {
            Some("CLAUDE_ASSETS_NOT_READY".to_string())
        },
        error_message: if registered {
            None
        } else {
            Some(format!(
                "Claude project assets are incomplete: {}",
                missing_items.join(", ")
            ))
        },
    }
}

/// Read current skill registration health using explicit settings and project root.
pub fn get_skill_registration_status_with_settings_at_project_root(
    settings: &Settings,
    project_root: Option<&Path>,
) -> SkillRegistrationStatus {
    let agents: Vec<SkillAgentRegistrationStatus> = SkillAgentType::all()
        .iter()
        .map(|a| status_for(*a, settings, project_root))
        .collect();

    let all_ok = agents.iter().all(|a| a.registered);
    let any_ok = agents.iter().any(|a| a.registered);

    let overall = if all_ok {
        "ok"
    } else if any_ok {
        "degraded"
    } else {
        "failed"
    };

    let last_error_message = agents
        .iter()
        .find_map(|a| a.error_message.clone())
        .or_else(|| {
            if all_ok {
                None
            } else {
                Some("Skill registration is incomplete.".to_string())
            }
        });

    SkillRegistrationStatus {
        overall: overall.to_string(),
        agents,
        last_checked_at: chrono::Utc::now().timestamp_millis(),
        last_error_message,
    }
}

/// Best-effort repair with explicit settings and project root.
pub fn repair_skill_registration_with_settings_at_project_root(
    settings: &Settings,
    project_root: Option<&Path>,
) -> SkillRegistrationStatus {
    if skill_registration_enabled(settings) {
        if let Err(err) = register_all_skills_with_settings_at_project_root(settings, project_root)
        {
            warn!(
                category = "skills",
                error = %err,
                "Failed to register one or more managed skills"
            );
        } else {
            info!(
                category = "skills",
                "Managed skills registered for all agents"
            );
        }
    } else {
        warn!(
            category = "skills",
            "Skipped skill registration because scope is not configured"
        );
    }

    get_skill_registration_status_with_settings_at_project_root(settings, project_root)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registration_settings() -> Settings {
        let mut settings = Settings::default();
        settings.agent.skill_registration =
            Some(crate::config::SkillRegistrationPreferences::default());
        settings
    }

    fn init_test_git_dir(root: &Path) {
        std::fs::create_dir_all(root.join(".git").join("info")).unwrap();
    }

    fn run_git(cwd: &Path, args: &[&str]) {
        let output = crate::process::command("git")
            .args(args)
            .current_dir(cwd)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "git {:?} failed in {}: {}",
            args,
            cwd.display(),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn registration_status_default_is_failed() {
        let status = SkillRegistrationStatus::default();
        assert_eq!(status.overall, "failed");
        assert_eq!(status.agents.len(), 3);
    }

    #[test]
    fn register_agent_skills_at_creates_expected_files() {
        let tmp = tempfile::tempdir().unwrap();
        register_agent_skills_at(tmp.path()).unwrap();

        for skill_name in MANAGED_SKILL_NAMES {
            let path = tmp.path().join("skills").join(skill_name).join("SKILL.md");
            assert!(path.exists(), "{} should exist", path.display());
        }

        assert!(tmp
            .path()
            .join("skills")
            .join("gwt-pr")
            .join("references")
            .join("pr-body-template.md")
            .exists());
        assert!(tmp
            .path()
            .join("skills")
            .join("gwt-spec-to-issue-migration")
            .join("scripts")
            .join("migrate-specs-to-issues.mjs")
            .exists());
        assert!(tmp
            .path()
            .join("skills")
            .join("gwt-pr-check")
            .join("scripts")
            .join("check_pr_status.py")
            .exists());
        assert!(tmp
            .path()
            .join("skills")
            .join("gwt-spec-ops")
            .join("scripts")
            .join("spec_artifact.py")
            .exists());
        assert!(tmp.path().join("memory").join("constitution.md").exists());
    }

    #[test]
    fn managed_skills_include_spec_to_issue_migration() {
        assert!(
            MANAGED_SKILL_NAMES.contains(&"gwt-spec-to-issue-migration"),
            "managed skills must include gwt-spec-to-issue-migration"
        );
        assert!(
            MANAGED_SKILL_NAMES.contains(&"gwt-issue-register"),
            "managed skills must include gwt-issue-register"
        );
    }

    #[test]
    fn managed_hook_detection_uses_exact_template_commands() {
        let managed_hook_commands =
            vec!["node .claude/hooks/scripts/gwt-forward-hook.mjs UserPromptSubmit".to_string()];

        assert!(is_managed_hook_command(
            "node .claude/hooks/scripts/gwt-forward-hook.mjs UserPromptSubmit",
            &managed_hook_commands
        ));
        assert!(!is_managed_hook_command(
            "echo gwt hook UserPromptSubmit",
            &managed_hook_commands
        ));
    }

    #[test]
    fn managed_hook_detection_accepts_legacy_commands_and_script_basenames() {
        let managed_hook_commands = Vec::new();

        assert!(is_managed_hook_command(
            "gwt hook UserPromptSubmit",
            &managed_hook_commands
        ));
        assert!(is_managed_hook_command(
            "/tmp/.claude/hooks/scripts/block-file-ops.sh",
            &managed_hook_commands
        ));
        assert!(is_managed_hook_command(
            "/tmp/.claude/hooks/scripts/forward-gwt-hook.sh Stop",
            &managed_hook_commands
        ));
    }

    #[test]
    fn prune_managed_hook_entries_preserves_user_hook_that_mentions_gwt_hook() {
        let managed_hook_commands =
            vec!["node .claude/hooks/scripts/gwt-forward-hook.mjs UserPromptSubmit".to_string()];
        let mut value = serde_json::json!(["echo gwt hook UserPromptSubmit"]);

        prune_managed_hook_entries(&mut value, &managed_hook_commands);

        assert_eq!(value, serde_json::json!(["echo gwt hook UserPromptSubmit"]));
    }

    #[test]
    fn status_for_reports_scope_not_configured_when_explicitly_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.agent.skill_registration = Some(crate::config::SkillRegistrationPreferences {
            enabled: false,
            ..Default::default()
        });
        let status = get_skill_registration_status_with_settings_at_project_root(
            &settings,
            Some(tmp.path()),
        );
        assert_eq!(status.overall, "failed");
        assert!(status.agents.iter().all(|agent| {
            agent.error_code.as_deref() == Some(SCOPE_NOT_CONFIGURED_CODE) && !agent.registered
        }));
    }

    #[test]
    fn skills_root_resolves_to_project_paths() {
        let temp = tempfile::tempdir().unwrap();

        let codex_path = skills_root_for(SkillAgentType::Codex, Some(temp.path())).unwrap();
        let gemini_path = skills_root_for(SkillAgentType::Gemini, Some(temp.path())).unwrap();
        let claude_path = skills_root_for(SkillAgentType::Claude, Some(temp.path()));

        assert_eq!(codex_path, temp.path().join(".codex").join("skills"));
        assert_eq!(gemini_path, temp.path().join(".gemini").join("skills"));
        assert!(claude_path.is_none(), "Claude uses .claude assets");
    }

    #[test]
    fn claude_settings_path_resolves_to_project_paths() {
        let temp = tempfile::tempdir().unwrap();
        let claude_path = claude_settings_path_for(Some(temp.path())).unwrap();
        assert_eq!(
            claude_path,
            temp.path().join(".claude").join("settings.local.json")
        );
    }

    #[test]
    fn register_all_skills_collects_agent_failures_when_project_root_missing() {
        let settings = registration_settings();
        let err = register_all_skills_with_settings_at_project_root(&settings, None)
            .expect_err("missing project root should return aggregated error");
        let reason = err.to_string();

        assert!(reason.contains("Codex"));
        assert!(reason.contains("Claude Code"));
        assert!(reason.contains("Gemini"));
    }

    #[test]
    fn register_with_settings_respects_explicit_disable() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.agent.skill_registration = Some(crate::config::SkillRegistrationPreferences {
            enabled: false,
            ..Default::default()
        });
        let result = register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Codex,
            &settings,
            Some(temp.path()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn register_with_default_settings_is_enabled() {
        let temp = tempfile::tempdir().unwrap();
        init_test_git_dir(temp.path());
        let result = register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Codex,
            &Settings::default(),
            Some(temp.path()),
        );
        assert!(result.is_ok());
        assert!(temp
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-issue-resolve")
            .join("SKILL.md")
            .exists());
    }

    #[test]
    fn project_scoped_registration_writes_all_agents() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();
        init_test_git_dir(temp.path());

        register_all_skills_with_settings_at_project_root(&settings, Some(temp.path())).unwrap();

        for skill_name in MANAGED_SKILL_NAMES {
            assert!(temp
                .path()
                .join(".codex")
                .join("skills")
                .join(skill_name)
                .join("SKILL.md")
                .exists());
            assert!(temp
                .path()
                .join(".gemini")
                .join("skills")
                .join(skill_name)
                .join("SKILL.md")
                .exists());
        }

        assert!(temp
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-pr")
            .join("references")
            .join("pr-body-template.md")
            .exists());
        assert!(temp
            .path()
            .join(".gemini")
            .join("skills")
            .join("gwt-spec-to-issue-migration")
            .join("scripts")
            .join("migrate-specs-to-issues.mjs")
            .exists());
        assert!(temp
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-pr-check")
            .join("scripts")
            .join("check_pr_status.py")
            .exists());
        assert!(temp
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-spec-ops")
            .join("scripts")
            .join("spec_artifact.py")
            .exists());
        assert!(temp.path().join("memory").join("constitution.md").exists());
        assert!(temp
            .path()
            .join(".gemini")
            .join("skills")
            .join("gwt-pr-check")
            .join("scripts")
            .join("check_pr_status.py")
            .exists());

        assert!(temp
            .path()
            .join(".claude")
            .join("commands")
            .join("gwt-pr.md")
            .exists());
        assert!(temp
            .path()
            .join(".claude")
            .join("skills")
            .join("gwt-pr")
            .join("SKILL.md")
            .exists());
        assert!(temp
            .path()
            .join(".claude")
            .join("hooks")
            .join("scripts")
            .join("gwt-forward-hook.mjs")
            .exists());
        assert!(!temp
            .path()
            .join(".claude")
            .join("hooks")
            .join("hooks.json")
            .exists());

        let settings_path = temp.path().join(".claude").join("settings.local.json");
        let content = std::fs::read_to_string(settings_path).unwrap();
        assert!(content.contains("gwt-forward-hook.mjs"));
        assert!(content.contains("gwt-block-git-branch-ops.mjs"));
        assert!(!content.contains("CLAUDE_PLUGIN_ROOT"));

        let exclude =
            std::fs::read_to_string(temp.path().join(".git").join("info").join("exclude")).unwrap();
        assert!(exclude.contains(PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_BEGIN_MARKER));
        assert!(exclude.contains(PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_END_MARKER));
        assert!(exclude.contains("/.codex/skills/gwt-*/"));
        assert!(exclude.contains("/.gemini/skills/gwt-*/"));
        assert!(exclude.contains("/.claude/skills/gwt-*/"));
        assert!(exclude.contains("/.claude/commands/gwt-*.md"));
        assert!(exclude.contains("/.claude/hooks/scripts/gwt-*.mjs"));
        assert!(exclude.contains("/.claude/settings.local.json"));
    }

    #[test]
    fn project_scoped_registration_recovers_from_legacy_global_profiles_schema() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _env = crate::config::TestEnvGuard::new(home.path());
        let project = tempfile::tempdir().unwrap();
        init_test_git_dir(project.path());

        let global_dir = home.path().join(".gwt");
        std::fs::create_dir_all(&global_dir).unwrap();
        std::fs::write(
            global_dir.join("config.toml"),
            r#"
[agent.skill_registration]
enabled = true

[profiles]
version = 1
active = "default"

[profiles.profiles.default]
name = "default"
disabled_env = []
description = ""

[profiles.profiles.default.env]
OPENAI_API_KEY = "legacy-key"
"#,
        )
        .unwrap();

        let settings = Settings::load_global().unwrap();
        let status = repair_skill_registration_with_settings_at_project_root(
            &settings,
            Some(project.path()),
        );

        assert_eq!(status.overall, "ok");
        assert!(project
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-issue-resolve")
            .join("SKILL.md")
            .exists());
        assert!(project
            .path()
            .join(".claude")
            .join("settings.local.json")
            .exists());
    }

    #[test]
    fn registration_rewrites_project_root_references_for_all_agents() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();

        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Claude,
            &settings,
            Some(temp.path()),
        )
        .unwrap();

        let skill_content = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("skills")
                .join("gwt-pr")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(!skill_content.contains("CLAUDE_PLUGIN_ROOT"));
        assert!(skill_content.contains(".claude/skills/gwt-pr/references/pr-body-template.md"));

        let command_content = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-pr.md"),
        )
        .unwrap();
        assert!(command_content.contains("`.claude/skills/gwt-pr/SKILL.md`"));

        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Codex,
            &settings,
            Some(temp.path()),
        )
        .unwrap();
        let codex_skill_content = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-pr")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(!codex_skill_content.contains("CLAUDE_PLUGIN_ROOT"));
        assert!(codex_skill_content.contains(".codex/skills/gwt-pr/references/pr-body-template.md"));
        assert!(codex_skill_content.contains("repos/<owner>/<repo>/pulls"));

        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Gemini,
            &settings,
            Some(temp.path()),
        )
        .unwrap();
        let gemini_skill_content = std::fs::read_to_string(
            temp.path()
                .join(".gemini")
                .join("skills")
                .join("gwt-spec-to-issue-migration")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(!gemini_skill_content.contains("CLAUDE_PLUGIN_ROOT"));
        assert!(gemini_skill_content.contains(
            ".gemini/skills/gwt-spec-to-issue-migration/scripts/migrate-specs-to-issues.mjs"
        ));
    }

    #[test]
    fn pr_assets_encode_upstream_first_post_merge_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();

        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Codex,
            &settings,
            Some(temp.path()),
        )
        .unwrap();
        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Claude,
            &settings,
            Some(temp.path()),
        )
        .unwrap();

        let codex_pr_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-pr")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(codex_pr_skill.contains("git merge-base --is-ancestor <merge_commit> HEAD"));
        assert!(codex_pr_skill.contains("git rev-list --count origin/<head>..HEAD"));
        assert!(codex_pr_skill.contains("MANUAL CHECK"));
        assert!(codex_pr_skill.contains("repos/<owner>/<repo>/pulls"));

        let claude_pr_skill = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("skills")
                .join("gwt-pr")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(claude_pr_skill.contains("git merge-base --is-ancestor <merge_commit> HEAD"));
        assert!(claude_pr_skill.contains("git rev-list --count origin/<head>..HEAD"));
        assert!(claude_pr_skill.contains("MANUAL CHECK"));

        let claude_pr_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-pr.md"),
        )
        .unwrap();
        assert!(claude_pr_command
            .contains("compare `origin/<head>..HEAD` before any base-branch fallback."));
        assert!(claude_pr_command
            .contains("merge `origin/$base` into the current branch and push before PR creation."));
        assert!(claude_pr_command.contains("REST pull-request endpoint"));

        let claude_pr_check_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-pr-check.md"),
        )
        .unwrap();
        assert!(claude_pr_check_command
            .contains("compare `origin/<head>..HEAD` before any base-branch fallback."));
        assert!(claude_pr_check_command
            .contains("return `MANUAL CHECK` instead of inferring `CREATE PR`."));
    }

    #[test]
    fn project_index_and_spec_ops_assets_encode_issue_search_first_guidance() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();

        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Codex,
            &settings,
            Some(temp.path()),
        )
        .unwrap();
        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Claude,
            &settings,
            Some(temp.path()),
        )
        .unwrap();

        let issue_register_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-issue-register")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(issue_register_skill.contains("Search existing Issues and `gwt-spec` Issues first"));
        assert!(issue_register_skill.contains("gwt-issue-search"));
        assert!(issue_register_skill.contains("gwt-spec-register"));
        assert!(issue_register_skill.contains("gwt-issue-resolve"));

        let project_index_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-project-index")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(project_index_skill.contains("project source files"));
        assert!(project_index_skill.contains("File search command"));
        assert!(!project_index_skill.contains("Issues search first"));

        let issue_search_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-issue-search")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(issue_search_skill.contains("Issues search first"));
        assert!(issue_search_skill.contains("spec integration"));
        assert!(issue_search_skill.contains("search-issues"));

        let issue_spec_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-spec-ops")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(issue_spec_skill.contains("search existing spec first"));
        assert!(issue_spec_skill.contains("gwt-issue-search"));

        let issue_resolve_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-issue-resolve")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(issue_resolve_skill.contains("Direct fix path"));
        assert!(issue_resolve_skill.contains("gwt-issue-search"));

        let spec_register_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-spec-register")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(spec_register_skill.contains("Issue-first SPEC container"));
        assert!(spec_register_skill.contains("gwt-issue-search"));
        assert!(spec_register_skill.contains("GWT_SPEC_ARTIFACT:doc:spec.md"));
        assert!(spec_register_skill.contains("gwt-spec-ops"));
        assert!(spec_register_skill.contains("repos/<owner>/<repo>/issues"));

        let spec_clarify_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-spec-clarify")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(spec_clarify_skill.contains("[NEEDS CLARIFICATION"));
        assert!(spec_clarify_skill.contains("gwt-spec-ops"));

        let spec_plan_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-spec-plan")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(spec_plan_skill.contains("memory/constitution.md"));
        assert!(spec_plan_skill.contains("Constitution Check"));
        assert!(spec_plan_skill.contains("gwt-spec-tasks"));

        let spec_tasks_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-spec-tasks")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(spec_tasks_skill.contains("[P]"));
        assert!(spec_tasks_skill.contains("user story"));
        assert!(spec_tasks_skill.contains("gwt-spec-analyze"));

        let spec_analyze_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-spec-analyze")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(spec_analyze_skill.contains("Status: CLEAR | AUTO-FIXABLE | NEEDS-DECISION"));
        assert!(spec_analyze_skill.contains("Constitution"));
        assert!(spec_analyze_skill.contains("gwt-spec-implement"));

        let spec_implement_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-spec-implement")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(spec_implement_skill.contains("implementation owner"));
        assert!(spec_implement_skill.contains("gwt-pr"));
        assert!(spec_implement_skill.contains("gwt-pr-fix"));

        let issue_register_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-issue-register.md"),
        )
        .unwrap();
        assert!(issue_register_command.contains("main entrypoint for new work registration"));
        assert!(issue_register_command.contains("gwt-issue-search"));
        assert!(issue_register_command.contains("gwt-spec-register"));
        assert!(issue_register_command.contains("gwt-spec-ops"));
        assert!(issue_register_command.contains("POST /repos/<owner>/<repo>/issues"));

        let issue_resolve_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-issue-resolve.md"),
        )
        .unwrap();
        assert!(issue_resolve_command.contains("direct fix"));
        assert!(issue_resolve_command.contains("existing SPEC"));
        assert!(issue_resolve_command.contains("gwt-issue-register"));
        assert!(issue_resolve_command.contains("gwt-spec-ops"));

        let spec_register_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-spec-register.md"),
        )
        .unwrap();
        assert!(spec_register_command.contains("seed `doc:spec.md`"));
        assert!(spec_register_command.contains("gwt-issue-search"));
        assert!(spec_register_command.contains("gwt-issue-register"));
        assert!(spec_register_command.contains("gwt-spec-ops"));

        let spec_clarify_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-spec-clarify.md"),
        )
        .unwrap();
        assert!(spec_clarify_command.contains("NEEDS CLARIFICATION"));
        assert!(spec_clarify_command.contains("gwt-spec-ops"));

        let spec_plan_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-spec-plan.md"),
        )
        .unwrap();
        assert!(spec_plan_command.contains("memory/constitution.md"));
        assert!(spec_plan_command.contains("gwt-spec-tasks"));
        assert!(spec_plan_command.contains("gwt-spec-ops"));

        let spec_tasks_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-spec-tasks.md"),
        )
        .unwrap();
        assert!(spec_tasks_command.contains("gwt-spec-analyze"));
        assert!(spec_tasks_command.contains("gwt-spec-ops"));

        let spec_analyze_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-spec-analyze.md"),
        )
        .unwrap();
        assert!(spec_analyze_command.contains("AUTO-FIXABLE"));
        assert!(spec_analyze_command.contains("gwt-spec-ops"));

        let spec_implement_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-spec-implement.md"),
        )
        .unwrap();
        assert!(spec_implement_command.contains("execution-ready"));
        assert!(spec_implement_command.contains("gwt-pr"));

        assert!(!temp
            .path()
            .join(".claude")
            .join("commands")
            .join("gwt-spec-ops.md")
            .exists());
    }

    #[test]
    fn registration_removes_retired_issue_assets() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();

        let codex_issue_ops_dir = temp
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-issue-ops");
        std::fs::create_dir_all(&codex_issue_ops_dir).unwrap();
        std::fs::write(codex_issue_ops_dir.join("SKILL.md"), "legacy").unwrap();

        let claude_commands_dir = temp.path().join(".claude").join("commands");
        std::fs::create_dir_all(&claude_commands_dir).unwrap();
        std::fs::write(claude_commands_dir.join("gwt-issue-ops.md"), "legacy").unwrap();
        std::fs::write(claude_commands_dir.join("gwt-spec-ops.md"), "legacy").unwrap();
        std::fs::write(claude_commands_dir.join("gwt-fix-pr.md"), "legacy").unwrap();

        let codex_fix_pr_dir = temp.path().join(".codex").join("skills").join("gwt-fix-pr");
        std::fs::create_dir_all(codex_fix_pr_dir.join("scripts")).unwrap();
        std::fs::write(codex_fix_pr_dir.join("SKILL.md"), "legacy").unwrap();
        std::fs::write(
            codex_fix_pr_dir
                .join("scripts")
                .join("inspect_pr_checks.py"),
            "legacy",
        )
        .unwrap();

        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Codex,
            &settings,
            Some(temp.path()),
        )
        .unwrap();
        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Claude,
            &settings,
            Some(temp.path()),
        )
        .unwrap();

        assert!(!temp
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-issue-ops")
            .exists());
        assert!(!temp
            .path()
            .join(".claude")
            .join("commands")
            .join("gwt-issue-ops.md")
            .exists());
        assert!(!temp
            .path()
            .join(".claude")
            .join("commands")
            .join("gwt-spec-ops.md")
            .exists());
        assert!(!temp
            .path()
            .join(".claude")
            .join("commands")
            .join("gwt-fix-pr.md")
            .exists());
        assert!(!temp
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-fix-pr")
            .exists());
        assert!(temp
            .path()
            .join(".codex")
            .join("skills")
            .join("gwt-issue-resolve")
            .join("SKILL.md")
            .exists());
    }

    #[test]
    fn claude_registration_writes_local_assets_even_when_plugin_is_enabled() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();
        let claude_root = temp.path().join(".claude");
        std::fs::create_dir_all(claude_root.join("commands")).unwrap();
        std::fs::create_dir_all(claude_root.join("skills").join("gwt-pr")).unwrap();
        std::fs::create_dir_all(claude_root.join("hooks").join("scripts")).unwrap();
        std::fs::write(claude_root.join("commands").join("gwt-pr.md"), "legacy").unwrap();
        std::fs::write(
            claude_root.join("skills").join("gwt-pr").join("SKILL.md"),
            "legacy",
        )
        .unwrap();
        std::fs::write(
            claude_root
                .join("hooks")
                .join("scripts")
                .join("forward-gwt-hook.sh"),
            "legacy",
        )
        .unwrap();
        std::fs::write(
            claude_root.join("settings.local.json"),
            serde_json::json!({
                "enabledPlugins": {
                    super::super::claude_plugins::GWT_PLUGIN_FULL_NAME: true
                },
                "hooks": {
                    "UserPromptSubmit": [
                        {
                            "hooks": [
                                {
                                    "type": "command",
                                    "command": "node .claude/hooks/scripts/gwt-forward-hook.mjs UserPromptSubmit"
                                }
                            ]
                        }
                    ]
                }
            })
            .to_string(),
        )
        .unwrap();

        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Claude,
            &settings,
            Some(temp.path()),
        )
        .unwrap();

        assert!(claude_root.join("commands").join("gwt-pr.md").exists());
        assert!(claude_root
            .join("skills")
            .join("gwt-pr")
            .join("SKILL.md")
            .exists());
        assert!(!claude_root.join("hooks").join("hooks.json").exists());
        assert!(!claude_root
            .join("hooks")
            .join("scripts")
            .join("forward-gwt-hook.sh")
            .exists());
        assert!(claude_root
            .join("hooks")
            .join("scripts")
            .join("gwt-forward-hook.mjs")
            .exists());
        assert!(temp.path().join("memory").join("constitution.md").exists());

        let settings_content =
            std::fs::read_to_string(claude_root.join("settings.local.json")).unwrap();
        assert!(settings_content.contains(super::super::claude_plugins::GWT_PLUGIN_FULL_NAME));

        let status = status_for_claude(Some(temp.path()));
        assert!(status.registered);
        assert!(status.missing_skills.is_empty());
    }

    #[test]
    fn claude_registration_keeps_gwt_pr_branch_preflight_rule() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();

        register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Claude,
            &settings,
            Some(temp.path()),
        )
        .unwrap();

        let skill_content = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("skills")
                .join("gwt-pr")
                .join("SKILL.md"),
        )
        .unwrap();

        assert!(skill_content.contains("git rev-list --left-right --count \"HEAD...origin/$base\""));
        assert!(skill_content.contains("git merge \"origin/$base\""));
        assert!(skill_content.contains("The update strategy is always `git merge origin/$base`; do not use rebase for this workflow."));
    }

    #[test]
    fn claude_registration_propagates_invalid_settings_local_json() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();
        let claude_root = temp.path().join(".claude");
        std::fs::create_dir_all(&claude_root).unwrap();
        std::fs::write(claude_root.join("settings.local.json"), "{invalid").unwrap();

        let err = register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Claude,
            &settings,
            Some(temp.path()),
        )
        .expect_err("invalid settings.local.json should abort registration");

        let reason = err.to_string();
        assert!(
            reason.contains("expected") || reason.contains("EOF") || reason.contains("key"),
            "unexpected error: {reason}"
        );
    }

    #[test]
    fn status_reports_path_unavailable_without_project_root() {
        let settings = registration_settings();
        let status = get_skill_registration_status_with_settings_at_project_root(&settings, None);

        assert_eq!(status.overall, "failed");
        assert!(status
            .agents
            .iter()
            .all(|agent| { agent.error_code.as_deref() == Some(SKILLS_PATH_UNAVAILABLE_CODE) }));
    }

    #[test]
    fn unregister_removes_skill_dirs_helper() {
        let tmp = tempfile::tempdir().unwrap();
        register_agent_skills_at(tmp.path()).unwrap();

        unregister_agent_skills_at(tmp.path());

        for skill_name in MANAGED_SKILL_NAMES {
            assert!(!tmp.path().join("skills").join(skill_name).exists());
        }
    }

    #[test]
    fn repair_uses_project_root_for_status() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();
        init_test_git_dir(temp.path());

        let status =
            repair_skill_registration_with_settings_at_project_root(&settings, Some(temp.path()));

        assert_eq!(status.overall, "ok");
    }

    #[test]
    fn exclude_rules_are_added_idempotently() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();
        init_test_git_dir(temp.path());

        register_all_skills_with_settings_at_project_root(&settings, Some(temp.path())).unwrap();
        register_all_skills_with_settings_at_project_root(&settings, Some(temp.path())).unwrap();

        let exclude =
            std::fs::read_to_string(temp.path().join(".git").join("info").join("exclude")).unwrap();
        assert_eq!(
            exclude
                .lines()
                .filter(|line| *line == PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_BEGIN_MARKER)
                .count(),
            1
        );
        assert_eq!(
            exclude
                .lines()
                .filter(|line| *line == PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_END_MARKER)
                .count(),
            1
        );
        for line in PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_LINES {
            assert_eq!(
                exclude.lines().filter(|existing| existing == line).count(),
                1,
                "exclude line should appear exactly once: {line}"
            );
        }
    }

    #[test]
    fn exclude_rules_preserve_non_gwt_entries_and_replace_legacy_entries() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();
        init_test_git_dir(temp.path());

        let exclude_path = temp.path().join(".git").join("info").join("exclude");
        std::fs::write(
            &exclude_path,
            "\
# custom rule
custom-pattern
/.codex/skills/gwt-*/**
# BEGIN gwt managed local assets
/.claude/skills/gwt-*/
# END gwt managed local assets
another-pattern
",
        )
        .unwrap();

        register_all_skills_with_settings_at_project_root(&settings, Some(temp.path())).unwrap();

        let exclude = std::fs::read_to_string(&exclude_path).unwrap();
        assert!(exclude.contains("# custom rule"));
        assert!(exclude.contains("custom-pattern"));
        assert!(exclude.contains("another-pattern"));
        assert!(!exclude.contains("/.codex/skills/gwt-*/**"));
        assert_eq!(
            exclude
                .lines()
                .filter(|line| *line == PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_BEGIN_MARKER)
                .count(),
            1
        );
        for line in PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_LINES {
            assert_eq!(
                exclude.lines().filter(|existing| existing == line).count(),
                1,
                "exclude line should appear exactly once: {line}"
            );
        }
    }

    #[test]
    fn linked_worktree_registration_writes_exclude_to_common_git_dir() {
        let temp = tempfile::tempdir().unwrap();
        let repo_root = temp.path().join("repo");
        let worktree_root = temp.path().join("worktree");

        run_git(temp.path(), &["init", repo_root.to_str().unwrap()]);
        run_git(&repo_root, &["config", "user.name", "Test User"]);
        run_git(&repo_root, &["config", "user.email", "test@example.com"]);
        std::fs::write(repo_root.join("README.md"), "test\n").unwrap();
        run_git(&repo_root, &["add", "README.md"]);
        run_git(&repo_root, &["commit", "-m", "test: init"]);
        run_git(
            &repo_root,
            &[
                "worktree",
                "add",
                "-b",
                "feature/test-worktree",
                worktree_root.to_str().unwrap(),
            ],
        );

        let settings = registration_settings();
        register_all_skills_with_settings_at_project_root(&settings, Some(&worktree_root)).unwrap();

        let exclude_path = git_path_for_project_root(&worktree_root, "info/exclude").unwrap();
        assert_eq!(
            dunce::canonicalize(&exclude_path).unwrap(),
            dunce::canonicalize(repo_root.join(".git").join("info").join("exclude")).unwrap()
        );
        let exclude = std::fs::read_to_string(exclude_path).unwrap();
        assert!(exclude.contains(PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_BEGIN_MARKER));
        assert!(exclude.contains("/.claude/hooks/scripts/gwt-*.mjs"));
    }

    // ── Skill catalog / managed block injection tests ──────────────

    #[test]
    fn generate_managed_skills_block_contains_all_skills() {
        let block = generate_managed_skills_block();

        // Every cataloged skill name must appear
        for entry in SKILL_CATALOG {
            assert!(
                block.contains(entry.name),
                "managed block should contain skill: {}",
                entry.name
            );
        }

        // Skills with commands should show `/gwt:<name>`
        for entry in SKILL_CATALOG.iter().filter(|e| e.has_command) {
            let command_ref = format!("/gwt:{}", entry.name);
            assert!(
                block.contains(&command_ref),
                "managed block should contain command ref: {command_ref}"
            );
        }

        // Skills without commands should show em-dash
        for entry in SKILL_CATALOG.iter().filter(|e| !e.has_command) {
            // Verify they do NOT have a `/gwt:` command reference
            let command_ref = format!("/gwt:{}", entry.name);
            assert!(
                !block.contains(&command_ref),
                "managed block should NOT contain command ref for no-command skill: {command_ref}"
            );
        }
    }

    #[test]
    fn inject_managed_skills_block_appends_to_content_without_block() {
        let existing = "# My CLAUDE.md\n\nSome existing content.\n";
        let result = inject_managed_skills_block(existing).unwrap();

        // Existing content must be preserved
        assert!(result.contains("# My CLAUDE.md"));
        assert!(result.contains("Some existing content."));

        // Managed block must be appended
        assert!(result.contains(MANAGED_SKILLS_BLOCK_BEGIN));
        assert!(result.contains(MANAGED_SKILLS_BLOCK_END));

        // Empty line separator between existing and managed block
        let begin_pos = result.find(MANAGED_SKILLS_BLOCK_BEGIN).unwrap();
        let before_begin = &result[..begin_pos];
        assert!(
            before_begin.ends_with("\n\n"),
            "managed block should be separated by an empty line"
        );
    }

    #[test]
    fn inject_managed_skills_block_replaces_existing_block() {
        let old_block = format!(
            "# Heading\n\nBefore.\n\n{}\nOld content\n{}\n\nAfter.\n",
            MANAGED_SKILLS_BLOCK_BEGIN, MANAGED_SKILLS_BLOCK_END
        );
        let result = inject_managed_skills_block(&old_block).unwrap();

        assert!(result.contains("# Heading"));
        assert!(result.contains("Before."));
        assert!(result.contains("After."));
        assert!(!result.contains("Old content"));
        assert!(result.contains(MANAGED_SKILLS_BLOCK_BEGIN));
        assert!(result.contains(MANAGED_SKILLS_BLOCK_END));
    }

    #[test]
    fn inject_managed_skills_block_is_idempotent() {
        let existing = "# My CLAUDE.md\n";
        let first = inject_managed_skills_block(existing).unwrap();
        let second = inject_managed_skills_block(&first).unwrap();

        assert_eq!(first, second, "inject must be idempotent");
    }

    #[test]
    fn inject_managed_skills_block_rejects_unterminated_begin() {
        let content = format!(
            "# Heading\n\n{}\nSome content\n",
            MANAGED_SKILLS_BLOCK_BEGIN
        );
        let result = inject_managed_skills_block(&content);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("END"),
            "error should mention missing END marker: {err}"
        );
    }

    #[test]
    fn inject_managed_skills_block_rejects_orphan_end() {
        let content = format!("# Heading\n\n{}\n", MANAGED_SKILLS_BLOCK_END);
        let result = inject_managed_skills_block(&content);

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("BEGIN"),
            "error should mention missing BEGIN marker: {err}"
        );
    }

    #[test]
    fn inject_managed_skills_block_handles_empty_content() {
        let result = inject_managed_skills_block("").unwrap();

        assert!(result.contains(MANAGED_SKILLS_BLOCK_BEGIN));
        assert!(result.contains(MANAGED_SKILLS_BLOCK_END));
        // Should be only the managed block (no leading separator for empty content)
        assert!(result.starts_with(MANAGED_SKILLS_BLOCK_BEGIN));
    }

    #[test]
    fn skill_catalog_matches_project_skill_assets() {
        // Every name in SKILL_CATALOG must correspond to a PROJECT_SKILL_ASSETS entry
        for entry in SKILL_CATALOG {
            let expected_relative = format!("skills/{}/SKILL.md", entry.name);
            let found = PROJECT_SKILL_ASSETS
                .iter()
                .any(|asset| asset.relative_path == expected_relative);
            assert!(
                found,
                "SKILL_CATALOG entry '{}' has no matching PROJECT_SKILL_ASSETS entry (expected relative_path='{}')",
                entry.name,
                expected_relative
            );
        }

        // Every unique skill name in PROJECT_SKILL_ASSETS (SKILL.md only) must appear in SKILL_CATALOG
        for asset in PROJECT_SKILL_ASSETS {
            if !asset.relative_path.ends_with("/SKILL.md") {
                continue;
            }
            let skill_name = asset
                .relative_path
                .strip_prefix("skills/")
                .and_then(|s| s.strip_suffix("/SKILL.md"))
                .unwrap();
            let found = SKILL_CATALOG.iter().any(|entry| entry.name == skill_name);
            assert!(
                found,
                "PROJECT_SKILL_ASSETS skill '{}' has no matching SKILL_CATALOG entry",
                skill_name
            );
        }
    }

    #[test]
    fn skill_registration_preferences_inject_defaults() {
        let prefs = crate::config::SkillRegistrationPreferences::default();
        assert!(prefs.enabled);
        assert!(prefs.inject_claude_md);
        assert!(!prefs.inject_agents_md);
        assert!(!prefs.inject_gemini_md);
    }

    #[test]
    fn exclude_rules_reject_unterminated_managed_block() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();
        init_test_git_dir(temp.path());

        let exclude_path = temp.path().join(".git").join("info").join("exclude");
        std::fs::write(
            &exclude_path,
            "\
# custom rule
custom-pattern
# BEGIN gwt managed local assets
/.claude/skills/gwt-*/
",
        )
        .unwrap();

        let err = register_all_skills_with_settings_at_project_root(&settings, Some(temp.path()))
            .expect_err("unterminated managed block should abort registration");
        assert!(
            err.to_string().contains("missing end marker"),
            "unexpected error: {err}"
        );
    }
}
