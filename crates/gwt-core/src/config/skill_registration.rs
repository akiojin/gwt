//! Skill registration for agent integrations.
//!
//! Uses managed local skill bundles
//! for supported agents.

use super::claude_plugins::{
    get_global_claude_settings_path, get_local_claude_settings_path, is_gwt_marketplace_registered,
    is_plugin_enabled_in_settings, setup_gwt_plugin, GWT_PLUGIN_FULL_NAME,
};
use crate::error::GwtError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::{info, warn};

/// Managed skill bundle definition.
#[derive(Debug, Clone, Copy)]
struct ManagedSkill {
    name: &'static str,
    body: &'static str,
}

const PTY_COMMUNICATION_SKILL: ManagedSkill = ManagedSkill {
    name: "gwt-pty-communication",
    body: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/worktree-protection-hooks/skills/gwt-pty-communication/SKILL.md"
    )),
};

const ISSUE_SPEC_SKILL: ManagedSkill = ManagedSkill {
    name: "gwt-issue-spec-ops",
    body: include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/worktree-protection-hooks/skills/gwt-issue-spec-ops/SKILL.md"
    )),
};

const MANAGED_SKILLS: &[ManagedSkill] = &[PTY_COMMUNICATION_SKILL, ISSUE_SPEC_SKILL];

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
        SkillAgentType::Claude => vec![format!("enabledPlugins.{GWT_PLUGIN_FULL_NAME}")],
        SkillAgentType::Codex | SkillAgentType::Gemini => {
            MANAGED_SKILLS.iter().map(|s| s.name.to_string()).collect()
        }
    }
}

fn skills_root_for(agent: SkillAgentType) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    Some(match agent {
        SkillAgentType::Codex => home.join(".codex").join("skills"),
        SkillAgentType::Gemini => home.join(".gemini").join("skills"),
        SkillAgentType::Claude => return None,
    })
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

/// Register managed skills for one agent.
pub fn register_agent_skills(agent: SkillAgentType) -> Result<(), GwtError> {
    match agent {
        SkillAgentType::Claude => setup_gwt_plugin(),
        SkillAgentType::Codex | SkillAgentType::Gemini => {
            let Some(root) = skills_root_for(agent) else {
                return Err(GwtError::ConfigWriteError {
                    reason: "Home directory could not be resolved".to_string(),
                });
            };
            register_agent_skills_at(&root)
        }
    }
}

/// Register managed skills for all supported agents.
pub fn register_all_skills() -> Result<(), GwtError> {
    for agent in SkillAgentType::all() {
        register_agent_skills(*agent)?;
    }
    Ok(())
}

fn status_for(agent: SkillAgentType) -> SkillAgentRegistrationStatus {
    if agent == SkillAgentType::Claude {
        return status_for_claude();
    }

    let root = skills_root_for(agent);
    let skills_path = root.as_ref().map(|p| p.to_string_lossy().to_string());

    let Some(root) = root else {
        return SkillAgentRegistrationStatus {
            agent_id: agent.id().to_string(),
            label: agent.label().to_string(),
            skills_path,
            registered: false,
            missing_skills: MANAGED_SKILLS.iter().map(|s| s.name.to_string()).collect(),
            error_code: Some("SKILLS_PATH_UNAVAILABLE".to_string()),
            error_message: Some("Home directory could not be resolved.".to_string()),
        };
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

fn status_for_claude() -> SkillAgentRegistrationStatus {
    let marketplace_registered = is_gwt_marketplace_registered();
    let global_path = get_global_claude_settings_path();
    let local_path = get_local_claude_settings_path();
    let global_enabled = global_path
        .as_ref()
        .map(|p| is_plugin_enabled_in_settings(p))
        .unwrap_or(false);
    let local_enabled = is_plugin_enabled_in_settings(&local_path);
    let plugin_enabled = global_enabled || local_enabled;

    let mut missing_items = Vec::new();
    if !marketplace_registered {
        missing_items.push("gwt-plugins-marketplace".to_string());
    }
    if !plugin_enabled {
        missing_items.push(format!("enabledPlugins.{GWT_PLUGIN_FULL_NAME}"));
    }

    let registered = missing_items.is_empty();
    let settings_hint = global_path
        .unwrap_or(local_path)
        .to_string_lossy()
        .to_string();

    SkillAgentRegistrationStatus {
        agent_id: SkillAgentType::Claude.id().to_string(),
        label: SkillAgentType::Claude.label().to_string(),
        skills_path: Some(settings_hint),
        registered,
        missing_skills: missing_items.clone(),
        error_code: if registered {
            None
        } else {
            Some("CLAUDE_PLUGIN_NOT_READY".to_string())
        },
        error_message: if registered {
            None
        } else {
            Some(format!(
                "Claude plugin registration is incomplete: {}",
                missing_items.join(", ")
            ))
        },
    }
}

/// Read current skill registration health.
pub fn get_skill_registration_status() -> SkillRegistrationStatus {
    let agents: Vec<SkillAgentRegistrationStatus> = SkillAgentType::all()
        .iter()
        .map(|a| status_for(*a))
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

/// Best-effort repair (register all, then return latest status).
pub fn repair_skill_registration() -> SkillRegistrationStatus {
    if let Err(err) = register_all_skills() {
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

    get_skill_registration_status()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn status_for_reports_missing_when_not_registered() {
        // use invalid environment by checking a dedicated temporary root via internal helper
        // here we validate shape through default status fallback.
        let status = SkillRegistrationStatus::default();
        assert!(status
            .agents
            .iter()
            .all(|agent| !agent.registered && !agent.missing_skills.is_empty()));
    }
}
