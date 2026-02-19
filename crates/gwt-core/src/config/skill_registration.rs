//! Skill registration for agent integrations.
//!
//! Uses managed local skill bundles
//! for supported agents.

use super::claude_plugins::{
    is_gwt_marketplace_registered, is_plugin_enabled_in_settings, setup_gwt_plugin_at,
    GWT_PLUGIN_FULL_NAME,
};
use super::{Settings, SkillRegistrationScope};
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
const SCOPE_NOT_CONFIGURED_CODE: &str = "SCOPE_NOT_CONFIGURED";
const SETTINGS_LOAD_FAILED_CODE: &str = "SETTINGS_LOAD_FAILED";
const SKILLS_PATH_UNAVAILABLE_CODE: &str = "SKILLS_PATH_UNAVAILABLE";
const SCOPE_NOT_CONFIGURED_MESSAGE: &str =
    "Skill registration scope is not configured. Open Settings and choose User, Project, or Local.";

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

fn project_root() -> Option<PathBuf> {
    std::env::current_dir().ok()
}

fn resolved_scope_for(
    agent: SkillAgentType,
    settings: &Settings,
) -> Option<SkillRegistrationScope> {
    let prefs = settings.agent.skill_registration.as_ref()?;
    Some(match agent {
        SkillAgentType::Codex => prefs.codex_scope.unwrap_or(prefs.default_scope),
        SkillAgentType::Claude => prefs.claude_scope.unwrap_or(prefs.default_scope),
        SkillAgentType::Gemini => prefs.gemini_scope.unwrap_or(prefs.default_scope),
    })
}

fn skills_root_for(agent: SkillAgentType, settings: &Settings) -> Option<PathBuf> {
    let scope = resolved_scope_for(agent, settings)?;
    match (agent, scope) {
        (SkillAgentType::Codex, SkillRegistrationScope::User) => {
            dirs::home_dir().map(|home| home.join(".codex").join("skills"))
        }
        (SkillAgentType::Gemini, SkillRegistrationScope::User) => {
            dirs::home_dir().map(|home| home.join(".gemini").join("skills"))
        }
        (SkillAgentType::Codex, SkillRegistrationScope::Project) => {
            project_root().map(|root| root.join(".codex").join("skills"))
        }
        (SkillAgentType::Gemini, SkillRegistrationScope::Project) => {
            project_root().map(|root| root.join(".gemini").join("skills"))
        }
        (SkillAgentType::Codex, SkillRegistrationScope::Local) => {
            project_root().map(|root| root.join(".codex").join("skills.local"))
        }
        (SkillAgentType::Gemini, SkillRegistrationScope::Local) => {
            project_root().map(|root| root.join(".gemini").join("skills.local"))
        }
        (SkillAgentType::Claude, _) => None,
    }
}

fn claude_settings_path_for(settings: &Settings) -> Option<PathBuf> {
    let scope = resolved_scope_for(SkillAgentType::Claude, settings)?;
    match scope {
        SkillRegistrationScope::User => {
            dirs::home_dir().map(|home| home.join(".claude").join("settings.json"))
        }
        SkillRegistrationScope::Project => {
            project_root().map(|root| root.join(".claude").join("settings.json"))
        }
        SkillRegistrationScope::Local => {
            project_root().map(|root| root.join(".claude").join("settings.local.json"))
        }
    }
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

/// Register managed skills for one agent with explicit settings.
pub fn register_agent_skills_with_settings(
    agent: SkillAgentType,
    settings: &Settings,
) -> Result<(), GwtError> {
    if settings.agent.skill_registration.is_none() {
        return Err(GwtError::ConfigWriteError {
            reason: SCOPE_NOT_CONFIGURED_MESSAGE.to_string(),
        });
    }

    match agent {
        SkillAgentType::Claude => {
            let Some(path) = claude_settings_path_for(settings) else {
                return Err(GwtError::ConfigWriteError {
                    reason: "Claude settings path could not be resolved for selected scope."
                        .to_string(),
                });
            };
            setup_gwt_plugin_at(&path)
        }
        SkillAgentType::Codex | SkillAgentType::Gemini => {
            let Some(root) = skills_root_for(agent, settings) else {
                return Err(GwtError::ConfigWriteError {
                    reason: format!(
                        "{} skills path could not be resolved for selected scope.",
                        agent.label()
                    ),
                });
            };
            register_agent_skills_at(&root)
        }
    }
}

/// Register managed skills for one agent.
pub fn register_agent_skills(agent: SkillAgentType) -> Result<(), GwtError> {
    let settings = Settings::load_global()?;
    register_agent_skills_with_settings(agent, &settings)
}

/// Register managed skills for all supported agents with explicit settings.
pub fn register_all_skills_with_settings(settings: &Settings) -> Result<(), GwtError> {
    for agent in SkillAgentType::all() {
        register_agent_skills_with_settings(*agent, settings)?;
    }
    Ok(())
}

/// Register managed skills for all supported agents.
pub fn register_all_skills() -> Result<(), GwtError> {
    let settings = Settings::load_global()?;
    register_all_skills_with_settings(&settings)
}

fn status_for(agent: SkillAgentType, settings: &Settings) -> SkillAgentRegistrationStatus {
    if settings.agent.skill_registration.is_none() {
        return scope_unconfigured_status(agent);
    }

    if agent == SkillAgentType::Claude {
        return status_for_claude(settings);
    }

    let root = skills_root_for(agent, settings);
    let skills_path = root.as_ref().map(|p| p.to_string_lossy().to_string());

    let Some(root) = root else {
        return path_unavailable_status(
            agent,
            "Skills path could not be resolved. Confirm current working directory and scope.",
            skills_path,
        );
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

fn status_for_claude(settings: &Settings) -> SkillAgentRegistrationStatus {
    let settings_path = claude_settings_path_for(settings);
    let Some(settings_path) = settings_path else {
        return path_unavailable_status(
            SkillAgentType::Claude,
            "Claude settings path could not be resolved. Confirm current working directory and scope.",
            None,
        );
    };

    let marketplace_registered = is_gwt_marketplace_registered();
    let plugin_enabled = is_plugin_enabled_in_settings(&settings_path);

    let mut missing_items = Vec::new();
    if !marketplace_registered {
        missing_items.push("gwt-plugins-marketplace".to_string());
    }
    if !plugin_enabled {
        missing_items.push(format!("enabledPlugins.{GWT_PLUGIN_FULL_NAME}"));
    }

    let registered = missing_items.is_empty();

    SkillAgentRegistrationStatus {
        agent_id: SkillAgentType::Claude.id().to_string(),
        label: SkillAgentType::Claude.label().to_string(),
        skills_path: Some(settings_path.to_string_lossy().to_string()),
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

fn settings_load_failure_status(error: &str) -> SkillRegistrationStatus {
    let message = format!("Failed to load settings for skill registration: {error}");
    let agents = SkillAgentType::all()
        .iter()
        .map(|agent| SkillAgentRegistrationStatus {
            agent_id: agent.id().to_string(),
            label: agent.label().to_string(),
            skills_path: None,
            registered: false,
            missing_skills: default_missing_items(*agent),
            error_code: Some(SETTINGS_LOAD_FAILED_CODE.to_string()),
            error_message: Some(message.clone()),
        })
        .collect();

    SkillRegistrationStatus {
        overall: "failed".to_string(),
        agents,
        last_checked_at: chrono::Utc::now().timestamp_millis(),
        last_error_message: Some(message),
    }
}

/// Read current skill registration health using explicit settings.
pub fn get_skill_registration_status_with_settings(settings: &Settings) -> SkillRegistrationStatus {
    let agents: Vec<SkillAgentRegistrationStatus> = SkillAgentType::all()
        .iter()
        .map(|a| status_for(*a, settings))
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

/// Read current skill registration health.
pub fn get_skill_registration_status() -> SkillRegistrationStatus {
    match Settings::load_global() {
        Ok(settings) => get_skill_registration_status_with_settings(&settings),
        Err(err) => settings_load_failure_status(&err.to_string()),
    }
}

/// Best-effort repair with explicit settings (register all, then return latest status).
pub fn repair_skill_registration_with_settings(settings: &Settings) -> SkillRegistrationStatus {
    if settings.agent.skill_registration.is_some() {
        if let Err(err) = register_all_skills_with_settings(settings) {
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

    get_skill_registration_status_with_settings(settings)
}

/// Best-effort repair (register all, then return latest status).
pub fn repair_skill_registration() -> SkillRegistrationStatus {
    match Settings::load_global() {
        Ok(settings) => repair_skill_registration_with_settings(&settings),
        Err(err) => {
            warn!(
                category = "skills",
                error = %err,
                "Failed to load settings during skill registration repair"
            );
            settings_load_failure_status(&err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CwdGuard {
        previous: PathBuf,
    }

    impl CwdGuard {
        fn set(path: &Path) -> Self {
            let previous = std::env::current_dir().unwrap();
            std::env::set_current_dir(path).unwrap();
            Self { previous }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous);
        }
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
    fn status_for_reports_scope_not_configured() {
        let status = get_skill_registration_status_with_settings(&Settings::default());
        assert_eq!(status.overall, "failed");
        assert!(status.agents.iter().all(|agent| {
            agent.error_code.as_deref() == Some(SCOPE_NOT_CONFIGURED_CODE) && !agent.registered
        }));
    }

    #[test]
    fn scope_resolution_uses_defaults_and_overrides() {
        let mut settings = Settings::default();
        settings.agent.skill_registration = Some(crate::config::SkillRegistrationPreferences {
            default_scope: SkillRegistrationScope::Project,
            codex_scope: Some(SkillRegistrationScope::User),
            claude_scope: None,
            gemini_scope: Some(SkillRegistrationScope::Local),
        });

        assert_eq!(
            resolved_scope_for(SkillAgentType::Codex, &settings),
            Some(SkillRegistrationScope::User)
        );
        assert_eq!(
            resolved_scope_for(SkillAgentType::Claude, &settings),
            Some(SkillRegistrationScope::Project)
        );
        assert_eq!(
            resolved_scope_for(SkillAgentType::Gemini, &settings),
            Some(SkillRegistrationScope::Local)
        );
    }

    #[test]
    fn resolves_scope_paths_for_all_agents() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let repo = temp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let _cwd = CwdGuard::set(&repo);

        let mut settings = Settings::default();
        settings.agent.skill_registration = Some(crate::config::SkillRegistrationPreferences {
            default_scope: SkillRegistrationScope::Project,
            codex_scope: Some(SkillRegistrationScope::User),
            claude_scope: Some(SkillRegistrationScope::Local),
            gemini_scope: Some(SkillRegistrationScope::Project),
        });

        let codex_path = skills_root_for(SkillAgentType::Codex, &settings).unwrap();
        let gemini_path = skills_root_for(SkillAgentType::Gemini, &settings).unwrap();
        let claude_path = claude_settings_path_for(&settings).unwrap();
        let canonical_repo = repo.canonicalize().unwrap();

        assert_eq!(codex_path, temp.path().join(".codex").join("skills"));
        assert_eq!(gemini_path, canonical_repo.join(".gemini").join("skills"));
        assert_eq!(
            claude_path,
            canonical_repo.join(".claude").join("settings.local.json")
        );
    }

    #[test]
    fn register_with_settings_requires_scope_configuration() {
        let result =
            register_agent_skills_with_settings(SkillAgentType::Codex, &Settings::default());
        assert!(result.is_err());
    }
}
