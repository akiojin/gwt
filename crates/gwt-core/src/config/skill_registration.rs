//! Skill/command/hook registration for agent integrations.
//!
//! Registration is project-scoped:
//! - Codex: `<project>/.codex/skills`
//! - Gemini: `<project>/.gemini/skills`
//! - Claude: `<project>/.claude/{skills,commands,hooks}` + `<project>/.claude/settings.json`

use super::Settings;
use crate::error::GwtError;
use crate::process::command;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Managed file asset definition for project-local agent assets.
#[derive(Debug, Clone, Copy)]
struct ManagedAsset {
    relative_path: &'static str,
    body: &'static str,
    executable: bool,
    rewrite_for_project: bool,
}

#[cfg(test)]
const MANAGED_SKILL_NAMES: &[&str] = &[
    "gwt-issue-ops",
    "gwt-fix-pr",
    "gwt-spec-ops",
    "gwt-pr",
    "gwt-pr-check",
    "gwt-project-index",
    "gwt-pty-communication",
    "gwt-spec-to-issue-migration",
];

const PROJECT_SKILL_ASSETS: &[ManagedAsset] = &[
    ManagedAsset {
        relative_path: "skills/gwt-issue-ops/SKILL.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-issue-ops/SKILL.md"
        )),
        executable: false,
        rewrite_for_project: true,
    },
    ManagedAsset {
        relative_path: "skills/gwt-issue-ops/scripts/inspect_issue.py",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-issue-ops/scripts/inspect_issue.py"
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
        relative_path: "skills/gwt-fix-pr/scripts/inspect_pr_checks.py",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-fix-pr/scripts/inspect_pr_checks.py"
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
        relative_path: "skills/gwt-spec-to-issue-migration/scripts/migrate-specs-to-issues.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/skills/gwt-spec-to-issue-migration/scripts/migrate-specs-to-issues.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
];

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
];

const CLAUDE_COMMAND_ASSETS: &[ManagedAsset] = &[
    ManagedAsset {
        relative_path: "commands/gwt-issue-ops.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-issue-ops.md"
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
        relative_path: "commands/gwt-spec-ops.md",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/commands/gwt-spec-ops.md"
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
        relative_path: "hooks/scripts/gwt-forward-hook.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-forward-hook.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-block-git-branch-ops.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-block-git-branch-ops.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-block-cd-command.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-block-cd-command.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-block-file-ops.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-block-file-ops.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
    ManagedAsset {
        relative_path: "hooks/scripts/gwt-block-git-dir-override.sh",
        body: include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../plugins/gwt/hooks/scripts/gwt-block-git-dir-override.sh"
        )),
        executable: true,
        rewrite_for_project: false,
    },
];

const LEGACY_MANAGED_ASSET_PATHS: &[&str] = &[
    "skills/gwt-fix-issue",
    "skills/gwt-issue-spec-ops",
    "commands/gwt-fix-issue.md",
    "commands/gwt-issue-spec-ops.md",
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
    "/.claude/hooks/scripts/gwt-*.sh",
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
    match agent {
        SkillAgentType::Claude => all_claude_assets()
            .map(|asset| format!(".claude/{}", asset.relative_path))
            .chain(std::iter::once(".claude/settings.json hooks".to_string()))
            .collect(),
        SkillAgentType::Codex => project_asset_missing_items(".codex"),
        SkillAgentType::Gemini => project_asset_missing_items(".gemini"),
    }
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
    claude_root_for(project_root).map(|root| root.join("settings.json"))
}

#[cfg(test)]
fn register_agent_skills_at(root: &Path) -> Result<(), GwtError> {
    write_managed_assets(root, PROJECT_SKILL_ASSETS.iter(), ".codex")
}

fn register_claude_assets_at(project_root: &Path) -> Result<(), GwtError> {
    let root = project_root.join(".claude");
    let settings_path = root.join("settings.json");

    let _ = super::claude_plugins::remove_gwt_plugin_key_at(&settings_path);
    super::claude_hooks::unregister_gwt_hooks(&settings_path)?;
    cleanup_legacy_claude_hook_scripts(&root)?;

    write_managed_assets(&root, all_claude_assets(), ".claude")?;
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
                    "command": ".claude/hooks/scripts/gwt-forward-hook.sh UserPromptSubmit"
                }]
            }],
            "PreToolUse": [
                {
                    "matcher": "*",
                    "hooks": [{
                        "type": "command",
                        "command": ".claude/hooks/scripts/gwt-forward-hook.sh PreToolUse"
                    }]
                },
                {
                    "matcher": "Bash",
                    "hooks": [
                        {
                            "type": "command",
                            "command": ".claude/hooks/scripts/gwt-block-git-branch-ops.sh"
                        },
                        {
                            "type": "command",
                            "command": ".claude/hooks/scripts/gwt-block-cd-command.sh"
                        },
                        {
                            "type": "command",
                            "command": ".claude/hooks/scripts/gwt-block-file-ops.sh"
                        },
                        {
                            "type": "command",
                            "command": ".claude/hooks/scripts/gwt-block-git-dir-override.sh"
                        }
                    ]
                }
            ],
            "PostToolUse": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": ".claude/hooks/scripts/gwt-forward-hook.sh PostToolUse"
                }]
            }],
            "Notification": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": ".claude/hooks/scripts/gwt-forward-hook.sh Notification"
                }]
            }],
            "Stop": [{
                "matcher": "*",
                "hooks": [{
                    "type": "command",
                    "command": ".claude/hooks/scripts/gwt-forward-hook.sh Stop"
                }]
            }]
        }
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

    let settings_path =
        claude_settings_path_for(project_root).unwrap_or_else(|| claude_root.join("settings.json"));

    let mut missing_items = Vec::new();

    for asset in all_claude_assets() {
        let asset_path = claude_root.join(asset.relative_path);
        if !asset_path.exists() {
            missing_items.push(format!(".claude/{}", asset.relative_path));
        }
    }

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
            .join("migrate-specs-to-issues.sh")
            .exists());
        assert!(tmp
            .path()
            .join("skills")
            .join("gwt-pr-check")
            .join("scripts")
            .join("check_pr_status.py")
            .exists());
    }

    #[test]
    fn managed_skills_include_spec_to_issue_migration() {
        assert!(
            MANAGED_SKILL_NAMES.contains(&"gwt-spec-to-issue-migration"),
            "managed skills must include gwt-spec-to-issue-migration"
        );
    }

    #[test]
    fn managed_hook_detection_uses_exact_template_commands() {
        let managed_hook_commands =
            vec![".claude/hooks/scripts/gwt-forward-hook.sh UserPromptSubmit".to_string()];

        assert!(is_managed_hook_command(
            ".claude/hooks/scripts/gwt-forward-hook.sh UserPromptSubmit",
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
            vec![".claude/hooks/scripts/gwt-forward-hook.sh UserPromptSubmit".to_string()];
        let mut value = serde_json::json!(["echo gwt hook UserPromptSubmit"]);

        prune_managed_hook_entries(&mut value, &managed_hook_commands);

        assert_eq!(value, serde_json::json!(["echo gwt hook UserPromptSubmit"]));
    }

    #[test]
    fn status_for_reports_scope_not_configured_when_explicitly_disabled() {
        let tmp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.agent.skill_registration =
            Some(crate::config::SkillRegistrationPreferences { enabled: false });
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
    fn register_with_settings_respects_explicit_disable() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.agent.skill_registration =
            Some(crate::config::SkillRegistrationPreferences { enabled: false });
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
            .join("gwt-issue-ops")
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
            .join("migrate-specs-to-issues.sh")
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
            .join("gwt-forward-hook.sh")
            .exists());
        assert!(!temp
            .path()
            .join(".claude")
            .join("hooks")
            .join("hooks.json")
            .exists());

        let settings_path = temp.path().join(".claude").join("settings.json");
        let content = std::fs::read_to_string(settings_path).unwrap();
        assert!(content.contains("gwt-forward-hook.sh"));
        assert!(content.contains("gwt-block-git-branch-ops.sh"));
        assert!(!content.contains("CLAUDE_PLUGIN_ROOT"));

        let exclude =
            std::fs::read_to_string(temp.path().join(".git").join("info").join("exclude")).unwrap();
        assert!(exclude.contains(PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_BEGIN_MARKER));
        assert!(exclude.contains(PROJECT_LOCAL_MANAGED_ASSET_EXCLUDE_END_MARKER));
        assert!(exclude.contains("/.codex/skills/gwt-*/"));
        assert!(exclude.contains("/.gemini/skills/gwt-*/"));
        assert!(exclude.contains("/.claude/skills/gwt-*/"));
        assert!(exclude.contains("/.claude/commands/gwt-*.md"));
        assert!(exclude.contains("/.claude/hooks/scripts/gwt-*.sh"));
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
            ".gemini/skills/gwt-spec-to-issue-migration/scripts/migrate-specs-to-issues.sh"
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
        assert!(claude_pr_command.contains("`MANUAL CHECK`"));

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

        let project_index_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-project-index")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(project_index_skill.contains("Issues search first"));
        assert!(project_index_skill.contains("spec integration"));
        assert!(project_index_skill.contains("search-issues"));

        let issue_spec_skill = std::fs::read_to_string(
            temp.path()
                .join(".codex")
                .join("skills")
                .join("gwt-spec-ops")
                .join("SKILL.md"),
        )
        .unwrap();
        assert!(issue_spec_skill.contains("search existing spec first"));
        assert!(issue_spec_skill.contains("gwt-project-index"));

        let issue_spec_command = std::fs::read_to_string(
            temp.path()
                .join(".claude")
                .join("commands")
                .join("gwt-spec-ops.md"),
        )
        .unwrap();
        assert!(issue_spec_command.contains("use `gwt-project-index` Issue search"));
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
            claude_root.join("settings.json"),
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
                                    "command": ".claude/hooks/scripts/gwt-forward-hook.sh UserPromptSubmit"
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
            .join("gwt-forward-hook.sh")
            .exists());

        let settings_content = std::fs::read_to_string(claude_root.join("settings.json")).unwrap();
        assert!(!settings_content.contains(super::super::claude_plugins::GWT_PLUGIN_FULL_NAME));

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
        assert!(skill_content.contains("Branch update required before creating a PR."));
    }

    #[test]
    fn claude_registration_propagates_invalid_settings_json() {
        let temp = tempfile::tempdir().unwrap();
        let settings = registration_settings();
        let claude_root = temp.path().join(".claude");
        std::fs::create_dir_all(&claude_root).unwrap();
        std::fs::write(claude_root.join("settings.json"), "{invalid").unwrap();

        let err = register_agent_skills_with_settings_at_project_root(
            SkillAgentType::Claude,
            &settings,
            Some(temp.path()),
        )
        .expect_err("invalid settings.json should abort registration");

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
        assert!(exclude.contains("/.claude/hooks/scripts/gwt-*.sh"));
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
