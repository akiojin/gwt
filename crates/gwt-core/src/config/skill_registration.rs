//! Skill/command/hook registration for agent integrations.
//!
//! Registration is project-scoped:
//! - Codex: `<project>/.codex/skills`
//! - Gemini: `<project>/.gemini/skills`
//! - Claude: `<project>/.claude/{skills,commands,hooks}` + `<project>/.claude/settings.json`

use super::Settings;
use crate::error::GwtError;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Managed skill bundle definition for Codex/Gemini.
#[derive(Debug, Clone, Copy)]
struct ManagedSkill {
    name: &'static str,
    body: &'static str,
}

/// Managed file asset definition for Claude project-local assets.
#[derive(Debug, Clone, Copy)]
struct ManagedAsset {
    relative_path: &'static str,
    body: &'static str,
    executable: bool,
    rewrite_for_project: bool,
}

const PTY_COMMUNICATION_SKILL: ManagedSkill = ManagedSkill {
    name: "gwt-pty-communication",
    body: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/gwt/skills/gwt-pty-communication/SKILL.md"
    )),
};

const ISSUE_SPEC_SKILL: ManagedSkill = ManagedSkill {
    name: "gwt-issue-spec-ops",
    body: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/gwt/skills/gwt-issue-spec-ops/SKILL.md"
    )),
};

const PROJECT_INDEX_SKILL: ManagedSkill = ManagedSkill {
    name: "gwt-project-index",
    body: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/gwt/skills/gwt-project-index/SKILL.md"
    )),
};

const SPEC_TO_ISSUE_MIGRATION_SKILL: ManagedSkill = ManagedSkill {
    name: "gwt-spec-to-issue-migration",
    body: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/gwt/skills/gwt-spec-to-issue-migration/SKILL.md"
    )),
};

const MANAGED_SKILLS: &[ManagedSkill] = &[
    PTY_COMMUNICATION_SKILL,
    ISSUE_SPEC_SKILL,
    PROJECT_INDEX_SKILL,
    SPEC_TO_ISSUE_MIGRATION_SKILL,
];

const CLAUDE_HOOKS_JSON_TEMPLATE: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../plugins/gwt/hooks/hooks.json"
));

const CLAUDE_COMMAND_ASSETS: &[ManagedAsset] = &[
    ManagedAsset {
        relative_path: "commands/gwt-fix-issue.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-fix-issue.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-fix-pr.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-fix-pr.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "commands/gwt-issue-spec-ops.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-issue-spec-ops.md"
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
        relative_path: "hooks/hooks.json",
        body: CLAUDE_HOOKS_JSON_TEMPLATE,
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/forward-gwt-hook.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/forward-gwt-hook.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/block-git-branch-ops.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/block-git-branch-ops.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/block-cd-command.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/block-cd-command.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/block-file-ops.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/block-file-ops.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/block-git-dir-override.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/block-git-dir-override.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
];

const CLAUDE_SKILL_ASSETS: &[ManagedAsset] = &[
    ManagedAsset {
        relative_path: "skills/gwt-fix-issue/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-fix-issue/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-fix-issue/scripts/inspect_issue.py",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-fix-issue/scripts/inspect_issue.py"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "skills/gwt-fix-pr/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-fix-pr/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-fix-pr/LICENSE.txt",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-fix-pr/LICENSE.txt"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "skills/gwt-fix-pr/scripts/inspect_pr_checks.py",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-fix-pr/scripts/inspect_pr_checks.py"
        )),
        executable: false,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "skills/gwt-issue-spec-ops/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-issue-spec-ops/SKILL.md"
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
];

const SCOPE_NOT_CONFIGURED_CODE: &str = "SCOPE_NOT_CONFIGURED";
const SKILLS_PATH_UNAVAILABLE_CODE: &str = "SKILLS_PATH_UNAVAILABLE";
const SCOPE_NOT_CONFIGURED_MESSAGE: &str =
    "Skill registration is not configured. Enable it in Settings.";
const PROJECT_ROOT_REQUIRED_MESSAGE: &str =
    "Project root is required for project-scoped skill registration.";

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
    match agent {
        SkillAgentType::Claude => vec![
            ".claude/hooks/hooks.json".to_string(),
            ".claude/commands/gwt-pr.md".to_string(),
            ".claude/skills/gwt-pty-communication/SKILL.md".to_string(),
            ".claude/settings.json hooks".to_string(),
        ],
        SkillAgentType::Codex | SkillAgentType::Gemini => {
            MANAGED_SKILLS.iter().map(|s| s.name.to_string()).collect()
        }
    }
}

fn skills_root_for(agent: SkillAgentType, project_root: Option<&Path>) -> Option<PathBuf> {
    let project_root = project_root?;
    match agent {
        SkillAgentType::Codex => Some(project_root.join(".codex").join("skills")),
        SkillAgentType::Gemini => Some(project_root.join(".gemini").join("skills")),
        SkillAgentType::Claude => None,
    }
}

fn claude_root_for(project_root: Option<&Path>) -> Option<PathBuf> {
    project_root.map(|root| root.join(".claude"))
}

fn claude_settings_path_for(project_root: Option<&Path>) -> Option<PathBuf> {
    claude_root_for(project_root).map(|root| root.join("settings.json"))
}

fn register_agent_skills_at(root: &Path) -> Result<(), GwtError> {
    std::fs::create_dir_all(root).map_err(|e| GwtError::ConfigWriteError {
        reason: format!("Failed to create skills root {}: {}", root.display(), e),
    })?;

    for skill in MANAGED_SKILLS {
        let dir = root.join(skill.name);
        std::fs::create_dir_all(&dir).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to create skill directory {}: {}", dir.display(), e),
        })?;

        let skill_file = dir.join("SKILL.md");
        std::fs::write(&skill_file, skill.body).map_err(|e| GwtError::ConfigWriteError {
            reason: format!("Failed to write skill file {}: {}", skill_file.display(), e),
        })?;
    }

    Ok(())
}

fn register_claude_assets_at(project_root: &Path) -> Result<(), GwtError> {
    let root = project_root.join(".claude");

    for asset in all_claude_assets() {
        write_claude_asset(&root, asset)?;
    }

    merge_managed_claude_hooks_into_settings(&root)
}

fn all_claude_assets() -> impl Iterator<Item = &'static ManagedAsset> {
    CLAUDE_COMMAND_ASSETS
        .iter()
        .chain(CLAUDE_HOOK_ASSETS.iter())
        .chain(CLAUDE_SKILL_ASSETS.iter())
}

fn write_claude_asset(root: &Path, asset: &ManagedAsset) -> Result<(), GwtError> {
    let path = root.join(asset.relative_path);
    let Some(parent) = path.parent() else {
        return Err(GwtError::ConfigWriteError {
            reason: format!("Invalid Claude asset path: {}", path.display()),
        });
    };

    std::fs::create_dir_all(parent).map_err(|e| GwtError::ConfigWriteError {
        reason: format!(
            "Failed to create Claude asset directory {}: {}",
            parent.display(),
            e
        ),
    })?;

    let content = if asset.rewrite_for_project {
        rewrite_claude_asset_content(asset.body)
    } else {
        asset.body.to_string()
    };

    std::fs::write(&path, content).map_err(|e| GwtError::ConfigWriteError {
        reason: format!("Failed to write Claude asset {}: {}", path.display(), e),
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

fn rewrite_claude_asset_content(content: &str) -> String {
    content
        .replace("${CLAUDE_PLUGIN_ROOT}", ".claude")
        .replace("$CLAUDE_PLUGIN_ROOT", ".claude")
        .replace("`skills/", "`.claude/skills/")
}

fn managed_hooks_definition() -> Result<Value, GwtError> {
    let rendered = rewrite_claude_asset_content(CLAUDE_HOOKS_JSON_TEMPLATE);
    serde_json::from_str::<Value>(&rendered).map_err(|e| GwtError::ConfigParseError {
        reason: format!("Failed to parse managed Claude hooks template: {e}"),
    })
}

fn merge_managed_claude_hooks_into_settings(claude_root: &Path) -> Result<(), GwtError> {
    let settings_path = claude_root.join("settings.json");

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
        let content = std::fs::read_to_string(&settings_path).unwrap_or_else(|_| "{}".to_string());
        serde_json::from_str::<Value>(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    if !settings.is_object() {
        settings = serde_json::json!({});
    }

    let hooks_definition = managed_hooks_definition()?;
    let Some(managed_hooks_map) = hooks_definition.get("hooks").and_then(|v| v.as_object()) else {
        return Err(GwtError::ConfigParseError {
            reason: "Managed Claude hooks template must have a hooks object".to_string(),
        });
    };

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
        prune_managed_hook_entries(value);
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

fn prune_managed_hook_entries(value: &mut Value) {
    let Some(entries) = value.as_array_mut() else {
        if value.as_str().map(is_managed_hook_command).unwrap_or(false) {
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
                    !is_managed_hook_command(command)
                });

                if hooks.is_empty() {
                    continue;
                }
            }

            retained_entries.push(entry);
            continue;
        }

        if let Some(command) = entry.as_str() {
            if is_managed_hook_command(command) {
                continue;
            }
        }

        retained_entries.push(entry);
    }

    *entries = retained_entries;
}

fn is_managed_hook_command(command: &str) -> bool {
    command.contains("gwt hook ")
        || command.contains("forward-gwt-hook.sh")
        || command.contains("block-git-branch-ops.sh")
        || command.contains("block-cd-command.sh")
        || command.contains("block-file-ops.sh")
        || command.contains("block-git-dir-override.sh")
}

fn required_managed_hook_commands() -> Vec<(String, String)> {
    let Ok(definition) = managed_hooks_definition() else {
        return Vec::new();
    };

    let Some(hooks_obj) = definition.get("hooks").and_then(|v| v.as_object()) else {
        return Vec::new();
    };

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
    for skill in MANAGED_SKILLS {
        let dir = root.join(skill.name);
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
    if settings.agent.skill_registration.is_none() {
        return Err(GwtError::ConfigWriteError {
            reason: SCOPE_NOT_CONFIGURED_MESSAGE.to_string(),
        });
    }

    let Some(project_root) = project_root else {
        return Err(GwtError::ConfigWriteError {
            reason: PROJECT_ROOT_REQUIRED_MESSAGE.to_string(),
        });
    };

    match agent {
        SkillAgentType::Claude => register_claude_assets_at(project_root),
        SkillAgentType::Codex | SkillAgentType::Gemini => {
            let Some(root) = skills_root_for(agent, Some(project_root)) else {
                return Err(GwtError::ConfigWriteError {
                    reason: format!("{} skills path could not be resolved.", agent.label()),
                });
            };
            register_agent_skills_at(&root)
        }
    }
}

/// Register managed skills for all supported agents with explicit settings and project root.
pub fn register_all_skills_with_settings_at_project_root(
    settings: &Settings,
    project_root: Option<&Path>,
) -> Result<(), GwtError> {
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
                "Failed to register skills/plugins for {} agent(s): {}",
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
    if settings.agent.skill_registration.is_none() {
        return scope_unconfigured_status(agent);
    }

    let Some(project_root) = project_root else {
        return path_unavailable_status(agent, PROJECT_ROOT_REQUIRED_MESSAGE, None);
    };

    if agent == SkillAgentType::Claude {
        return status_for_claude(Some(project_root));
    }

    let root = skills_root_for(agent, Some(project_root));
    let skills_path = root.as_ref().map(|p| p.to_string_lossy().to_string());

    let Some(root) = root else {
        return path_unavailable_status(agent, "Skills path could not be resolved.", skills_path);
    };

    let mut missing = Vec::new();
    for skill in MANAGED_SKILLS {
        let skill_file = root.join(skill.name).join("SKILL.md");
        if !skill_file.exists() {
            missing.push(skill.name.to_string());
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
            Some("SKILLS_MISSING".to_string())
        },
        error_message: if registered {
            None
        } else {
            Some(format!("Missing skills: {}", missing.join(", ")))
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

    let mut missing_items = Vec::new();

    for asset in all_claude_assets() {
        let asset_path = claude_root.join(asset.relative_path);
        if !asset_path.exists() {
            missing_items.push(format!(".claude/{}", asset.relative_path));
        }
    }

    let settings_path =
        claude_settings_path_for(project_root).unwrap_or_else(|| claude_root.join("settings.json"));
    if !settings_path.exists() {
        missing_items.push(".claude/settings.json".to_string());
    } else {
        for event in missing_managed_hook_events(&settings_path) {
            missing_items.push(format!(".claude/settings.json hooks.{event}"));
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
    if settings.agent.skill_registration.is_some() {
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
        settings.agent.skill_registration = Some(crate::config::SkillRegistrationPreferences {});
        settings
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

        for skill in MANAGED_SKILLS {
            let path = tmp.path().join(skill.name).join("SKILL.md");
            assert!(path.exists(), "{} should exist", path.display());
        }
    }

    #[test]
    fn managed_skills_include_spec_to_issue_migration() {
        assert!(
            MANAGED_SKILLS
                .iter()
                .any(|skill| skill.name == "gwt-spec-to-issue-migration"),
            "managed skills must include gwt-spec-to-issue-migration"
        );
    }

    #[test]
    fn status_for_reports_scope_not_configured() {
        let tmp = tempfile::tempdir().unwrap();
        let status = get_skill_registration_status_with_settings_at_project_root(
            &Settings::default(),
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
            temp.path().join(".claude").join("settings.json")
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
    fn register_with_settings_requires_scope_configuration() {
        let temp = tempfile::tempdir().unwrap();
        let result = register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Codex,
            &Settings::default(),
            Some(temp.path()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn project_scoped_registration_writes_all_agents() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();

        register_all_skills_with_settings_at_project_root(&settings, Some(temp.path())).unwrap();

        for skill in MANAGED_SKILLS {
            assert!(temp
                .path()
                .join(".codex")
                .join("skills")
                .join(skill.name)
                .join("SKILL.md")
                .exists());
            assert!(temp
                .path()
                .join(".gemini")
                .join("skills")
                .join(skill.name)
                .join("SKILL.md")
                .exists());
        }

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
            .join("forward-gwt-hook.sh")
            .exists());

        let settings_path = temp.path().join(".claude").join("settings.json");
        let content = std::fs::read_to_string(settings_path).unwrap();
        assert!(content.contains("forward-gwt-hook.sh"));
        assert!(content.contains("block-git-branch-ops.sh"));
        assert!(!content.contains("CLAUDE_PLUGIN_ROOT"));
    }

    #[test]
    fn claude_registration_rewrites_claude_plugin_root_references() {
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

        for skill in MANAGED_SKILLS {
            assert!(!tmp.path().join(skill.name).exists());
        }
    }

    #[test]
    fn repair_uses_project_root_for_status() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();

        let status =
            repair_skill_registration_with_settings_at_project_root(&settings, Some(temp.path()));

        assert_eq!(status.overall, "ok");
    }
}
