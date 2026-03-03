//! Skill registration for agent integrations.
//!
//! Uses managed local skill bundles
//! for supported agents.

use super::claude_plugins::{
    is_gwt_marketplace_registered, is_plugin_enabled_in_settings, setup_gwt_plugin_at,
    GWT_PLUGIN_FULL_NAME,
};
use super::Settings;
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
const SCOPE_NOT_CONFIGURED_CODE: &str = "SCOPE_NOT_CONFIGURED";
const SETTINGS_LOAD_FAILED_CODE: &str = "SETTINGS_LOAD_FAILED";
const SKILLS_PATH_UNAVAILABLE_CODE: &str = "SKILLS_PATH_UNAVAILABLE";
const SCOPE_NOT_CONFIGURED_MESSAGE: &str =
    "Skill registration is not configured. Enable it in Settings.";

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
    match agent {
        SkillAgentType::Codex => dirs::home_dir().map(|home| home.join(".codex").join("skills")),
        SkillAgentType::Gemini => dirs::home_dir().map(|home| home.join(".gemini").join("skills")),
        SkillAgentType::Claude => None,
    }
}

fn claude_settings_path_for() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".claude").join("settings.json"))
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

/// Remove managed skill directories for one agent at the given root.
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

/// Unregister all managed skills/plugins for every agent (best-effort).
///
/// - Claude: sets `enabledPlugins.gwt@gwt-plugins` to `false` in `~/.claude/settings.json`
/// - Codex: removes skill directories under `~/.codex/skills/`
/// - Gemini: removes skill directories under `~/.gemini/skills/`
///
/// Errors are logged but never propagated; this function is designed to be called
/// at application exit without blocking the shutdown sequence.
pub fn unregister_all_skills() {
    // Claude: disable plugin
    if let Some(settings_path) = super::claude_plugins::get_global_claude_settings_path() {
        if let Err(e) = super::claude_plugins::disable_gwt_plugin_at(&settings_path) {
            warn!(
                category = "skills",
                agent = "claude",
                error = %e,
                "Failed to disable Claude plugin on exit"
            );
        } else {
            info!(
                category = "skills",
                agent = "claude",
                "Disabled Claude plugin on exit"
            );
        }
    }

    // Codex: remove skill directories
    if let Some(home) = dirs::home_dir() {
        let codex_root = home.join(".codex").join("skills");
        unregister_agent_skills_at(&codex_root);
        info!(
            category = "skills",
            agent = "codex",
            "Unregistered Codex skills on exit"
        );
    }

    // Gemini: remove skill directories
    if let Some(home) = dirs::home_dir() {
        let gemini_root = home.join(".gemini").join("skills");
        unregister_agent_skills_at(&gemini_root);
        info!(
            category = "skills",
            agent = "gemini",
            "Unregistered Gemini skills on exit"
        );
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

/// Register managed skills for one agent with explicit settings.
pub fn register_agent_skills_with_settings(
    agent: SkillAgentType,
    settings: &Settings,
) -> Result<(), GwtError> {
    register_agent_skills_with_settings_at_project_root(agent, settings, None)
}

/// Register managed skills for one agent using an explicit project root.
///
/// The `project_root` parameter is retained for API compatibility but is no longer
/// used; registration always targets the User scope (home directory).
pub fn register_agent_skills_with_settings_at_project_root(
    agent: SkillAgentType,
    settings: &Settings,
    _project_root: Option<&Path>,
) -> Result<(), GwtError> {
    if settings.agent.skill_registration.is_none() {
        return Err(GwtError::ConfigWriteError {
            reason: SCOPE_NOT_CONFIGURED_MESSAGE.to_string(),
        });
    }

    match agent {
        SkillAgentType::Claude => {
            let Some(path) = claude_settings_path_for() else {
                return Err(GwtError::ConfigWriteError {
                    reason: "Claude settings path could not be resolved.".to_string(),
                });
            };
            setup_gwt_plugin_at(&path)
        }
        SkillAgentType::Codex | SkillAgentType::Gemini => {
            let Some(root) = skills_root_for(agent) else {
                return Err(GwtError::ConfigWriteError {
                    reason: format!(
                        "{} skills path could not be resolved.",
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
    register_all_skills_with_settings_at_project_root(settings, None)
}

/// Register managed skills for all supported agents with explicit settings and project root.
///
/// The `project_root` parameter is retained for API compatibility but is no longer
/// used; registration always targets the User scope (home directory).
pub fn register_all_skills_with_settings_at_project_root(
    settings: &Settings,
    _project_root: Option<&Path>,
) -> Result<(), GwtError> {
    let mut failures = Vec::new();
    for agent in SkillAgentType::all() {
        if let Err(err) =
            register_agent_skills_with_settings_at_project_root(*agent, settings, None)
        {
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

/// Register managed skills for all supported agents.
pub fn register_all_skills() -> Result<(), GwtError> {
    let settings = Settings::load_global()?;
    register_all_skills_with_settings(&settings)
}

fn status_for(
    agent: SkillAgentType,
    settings: &Settings,
) -> SkillAgentRegistrationStatus {
    if settings.agent.skill_registration.is_none() {
        return scope_unconfigured_status(agent);
    }

    if agent == SkillAgentType::Claude {
        return status_for_claude();
    }

    let root = skills_root_for(agent);
    let skills_path = root.as_ref().map(|p| p.to_string_lossy().to_string());

    let Some(root) = root else {
        return path_unavailable_status(
            agent,
            "Skills path could not be resolved.",
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

fn status_for_claude() -> SkillAgentRegistrationStatus {
    let settings_path = claude_settings_path_for();
    let Some(settings_path) = settings_path else {
        return path_unavailable_status(
            SkillAgentType::Claude,
            "Claude settings path could not be resolved.",
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
    get_skill_registration_status_with_settings_at_project_root(settings, None)
}

/// Read current skill registration health using explicit settings and project root.
///
/// The `project_root` parameter is retained for API compatibility but is no longer
/// used; status always checks the User scope (home directory).
pub fn get_skill_registration_status_with_settings_at_project_root(
    settings: &Settings,
    _project_root: Option<&Path>,
) -> SkillRegistrationStatus {
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
    repair_skill_registration_with_settings_at_project_root(settings, None)
}

/// Best-effort repair with explicit settings and project root.
///
/// The `project_root` parameter is retained for API compatibility but is no longer
/// used; repair always targets the User scope (home directory).
pub fn repair_skill_registration_with_settings_at_project_root(
    settings: &Settings,
    _project_root: Option<&Path>,
) -> SkillRegistrationStatus {
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
        let status = get_skill_registration_status_with_settings(&Settings::default());
        assert_eq!(status.overall, "failed");
        assert!(status.agents.iter().all(|agent| {
            agent.error_code.as_deref() == Some(SCOPE_NOT_CONFIGURED_CODE) && !agent.registered
        }));
    }

    #[test]
    fn skills_root_resolves_to_user_home() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let codex_path = skills_root_for(SkillAgentType::Codex).unwrap();
        let gemini_path = skills_root_for(SkillAgentType::Gemini).unwrap();
        let claude_path = skills_root_for(SkillAgentType::Claude);

        assert_eq!(codex_path, temp.path().join(".codex").join("skills"));
        assert_eq!(gemini_path, temp.path().join(".gemini").join("skills"));
        assert!(claude_path.is_none(), "Claude uses plugin, not skill dir");
    }

    #[test]
    fn claude_settings_path_resolves_to_user_home() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let claude_path = claude_settings_path_for().unwrap();
        assert_eq!(
            claude_path,
            temp.path().join(".claude").join("settings.json")
        );
    }

    #[test]
    fn register_all_skills_collects_agent_failures() {
        let err = register_all_skills_with_settings_at_project_root(&Settings::default(), None)
            .expect_err("missing scope should return aggregated error");
        let reason = err.to_string();

        assert!(reason.contains("Codex"));
        assert!(reason.contains("Claude Code"));
        assert!(reason.contains("Gemini"));
    }

    #[test]
    fn register_with_settings_requires_scope_configuration() {
        let result =
            register_agent_skills_with_settings(SkillAgentType::Codex, &Settings::default());
        assert!(result.is_err());
    }

    #[test]
    fn test_unregister_removes_skill_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        register_agent_skills_at(tmp.path()).unwrap();
        // Verify skills exist
        for skill in MANAGED_SKILLS {
            assert!(tmp.path().join(skill.name).join("SKILL.md").exists());
        }

        unregister_agent_skills_at(tmp.path());

        // Verify skills are removed
        for skill in MANAGED_SKILLS {
            assert!(!tmp.path().join(skill.name).exists());
        }
    }

    #[test]
    fn test_unregister_noop_no_skills() {
        let tmp = tempfile::tempdir().unwrap();
        // Should not panic or error on empty directory
        unregister_agent_skills_at(tmp.path());
    }

    #[test]
    fn test_unregister_claude_disables_plugin() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        let settings_path = temp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().unwrap()).unwrap();
        let content = format!(
            r#"{{"enabledPlugins": {{"{}": true}}}}"#,
            super::super::claude_plugins::GWT_PLUGIN_FULL_NAME
        );
        std::fs::write(&settings_path, content).unwrap();

        unregister_all_skills();

        let content = std::fs::read_to_string(&settings_path).unwrap();
        let settings: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            settings["enabledPlugins"][super::super::claude_plugins::GWT_PLUGIN_FULL_NAME],
            serde_json::json!(false)
        );
    }

    #[test]
    fn test_unregister_preserves_marketplace() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        // Create marketplace file
        let marketplace_path = temp
            .path()
            .join(".claude")
            .join("plugins")
            .join("known_marketplaces.json");
        std::fs::create_dir_all(marketplace_path.parent().unwrap()).unwrap();
        let marketplace_content = r#"{"gwt-plugins": {"source": {"source": "github", "repo": "akiojin/gwt"}, "installLocation": "/tmp/test", "lastUpdated": "2025-01-01T00:00:00.000Z"}}"#;
        std::fs::write(&marketplace_path, marketplace_content).unwrap();

        unregister_all_skills();

        // Marketplace should be untouched
        let content = std::fs::read_to_string(&marketplace_path).unwrap();
        assert!(content.contains("gwt-plugins"));
    }

    #[test]
    fn test_unregister_all_best_effort() {
        let _lock = crate::config::HOME_LOCK.lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _env = crate::config::TestEnvGuard::new(temp.path());

        // Create codex skills
        let codex_root = temp.path().join(".codex").join("skills");
        register_agent_skills_at(&codex_root).unwrap();

        // Create gemini skills
        let gemini_root = temp.path().join(".gemini").join("skills");
        register_agent_skills_at(&gemini_root).unwrap();

        // unregister_all_skills should succeed (best-effort, returns ())
        unregister_all_skills();

        // Both should be cleaned up
        for skill in MANAGED_SKILLS {
            assert!(
                !codex_root.join(skill.name).exists(),
                "codex skill {} should be removed",
                skill.name
            );
            assert!(
                !gemini_root.join(skill.name).exists(),
                "gemini skill {} should be removed",
                skill.name
            );
        }
    }
}
