//! Managed hook health read model.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use gwt_agent::PendingDiscussionResume;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const SLOW_HANDLER_THRESHOLD_MS: f64 = 1000.0;
const MANAGED_EVENTS: &[&str] = &[
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "Stop",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedHookHealthStatus {
    Ready,
    NeedsAttention,
    SelfHealed,
    Degraded,
    Inactive,
    WaitingForFirstHookEvent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PendingHookGoal {
    pub proposal_label: String,
    pub proposal_title: String,
    pub condition: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HookProfileEvidence {
    pub event: String,
    pub handler: String,
    pub status: String,
    pub duration_ms: f64,
    pub occurred_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ManagedHookHealth {
    pub status: ManagedHookHealthStatus,
    pub last_event: Option<String>,
    pub last_event_at: Option<String>,
    pub pending_discussion: Option<PendingDiscussionResume>,
    pub pending_goal: Option<PendingHookGoal>,
    pub slow_handlers: Vec<HookProfileEvidence>,
    pub issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedHookHealthInput {
    pub worktree_root: PathBuf,
    pub runtime_state_path: Option<PathBuf>,
    pub profile_path: Option<PathBuf>,
    pub expected_hook_bin: Option<String>,
}

impl ManagedHookHealthInput {
    pub fn new(worktree_root: impl AsRef<Path>) -> Self {
        Self {
            worktree_root: worktree_root.as_ref().to_path_buf(),
            runtime_state_path: None,
            profile_path: None,
            expected_hook_bin: None,
        }
    }

    pub fn with_runtime_state_path(mut self, path: impl AsRef<Path>) -> Self {
        self.runtime_state_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_profile_path(mut self, path: impl AsRef<Path>) -> Self {
        self.profile_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn with_expected_hook_bin(mut self, bin: impl Into<String>) -> Self {
        self.expected_hook_bin = Some(bin.into());
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ManagedHookRepairOutcome {
    pub repaired: bool,
}

#[derive(Debug, Deserialize)]
struct RuntimeStateReadModel {
    pub status: String,
    pub updated_at: String,
    #[allow(dead_code)]
    pub last_activity_at: String,
    #[serde(default)]
    pub source_event: Option<String>,
    #[serde(default)]
    pub pending_discussion: Option<PendingDiscussionResume>,
}

pub fn read_managed_hook_health(input: &ManagedHookHealthInput) -> ManagedHookHealth {
    let mut health = ManagedHookHealth {
        status: ManagedHookHealthStatus::Ready,
        last_event: None,
        last_event_at: None,
        pending_discussion: None,
        pending_goal: crate::discussion_resume::load_pending_goal_from_worktree_files(
            &input.worktree_root,
        )
        .ok()
        .flatten()
        .map(|goal| PendingHookGoal {
            proposal_label: goal.proposal_label,
            proposal_title: goal.proposal_title,
            condition: goal.condition,
        }),
        slow_handlers: Vec::new(),
        issues: Vec::new(),
    };

    audit_managed_hook_configs(input, &mut health);
    audit_hook_profile(input, &mut health);

    let Some(runtime_state_path) = input.runtime_state_path.as_ref() else {
        if health.status == ManagedHookHealthStatus::Ready {
            health.status = ManagedHookHealthStatus::Inactive;
        }
        return health;
    };

    if !runtime_state_path.exists() {
        if health.status == ManagedHookHealthStatus::Ready {
            health.status = ManagedHookHealthStatus::WaitingForFirstHookEvent;
        }
        return health;
    }

    match read_runtime_state(runtime_state_path) {
        Ok(runtime_state) => {
            if let Some(source_event) = runtime_state.source_event {
                health.last_event = Some(source_event);
                health.last_event_at = Some(runtime_state.updated_at);
            } else if health.status == ManagedHookHealthStatus::Ready {
                health.status = ManagedHookHealthStatus::WaitingForFirstHookEvent;
            }
            health.pending_discussion = runtime_state.pending_discussion;
            if runtime_state.status == "Stopped" && health.status == ManagedHookHealthStatus::Ready
            {
                health.status = ManagedHookHealthStatus::Inactive;
            }
        }
        Err(error) => {
            health.status = ManagedHookHealthStatus::Degraded;
            health
                .issues
                .push(format!("runtime state could not be read: {}", error));
        }
    }

    health
}

fn audit_hook_profile(input: &ManagedHookHealthInput, health: &mut ManagedHookHealth) {
    let Some(profile_path) = input.profile_path.as_ref() else {
        return;
    };
    if !profile_path.exists() {
        return;
    }

    let Ok(raw) = fs::read_to_string(profile_path) else {
        needs_attention(
            health,
            format!("hook profile could not be read: {}", profile_path.display()),
        );
        return;
    };

    for (index, line) in raw.lines().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(trimmed) else {
            needs_attention(
                health,
                format!(
                    "hook profile line {} is not valid JSON: {}",
                    index + 1,
                    profile_path.display()
                ),
            );
            continue;
        };
        let Some(duration_ms) = record.get("duration_ms").and_then(Value::as_f64) else {
            continue;
        };
        if duration_ms < SLOW_HANDLER_THRESHOLD_MS {
            continue;
        }
        let event = record
            .get("event")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let handler = record
            .get("handler")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let status = record
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let occurred_at = record
            .get("occurred_at")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        health.slow_handlers.push(HookProfileEvidence {
            event: event.clone(),
            handler: handler.clone(),
            status,
            duration_ms,
            occurred_at,
        });
        needs_attention(
            health,
            format!("slow managed hook handler: {event}/{handler} took {duration_ms:.1}ms"),
        );
    }
}

fn audit_managed_hook_configs(input: &ManagedHookHealthInput, health: &mut ManagedHookHealth) {
    let worktree = &input.worktree_root;
    let claude_dir = worktree.join(".claude");
    let claude_settings = worktree.join(".claude/settings.local.json");
    let codex_dir = worktree.join(".codex");
    let codex_hooks = worktree.join(".codex/hooks.json");

    let has_surface = claude_dir.exists()
        || claude_settings.exists()
        || codex_dir.exists()
        || codex_hooks.exists();
    if !has_surface {
        health.status = ManagedHookHealthStatus::Inactive;
        return;
    }

    if claude_dir.exists() && !claude_settings.exists() {
        needs_attention(
            health,
            "managed hook config missing: .claude/settings.local.json",
        );
    }
    if codex_dir.exists() && !codex_hooks.exists() {
        needs_attention(health, "managed hook config missing: .codex/hooks.json");
    }

    if claude_settings.exists() {
        audit_hook_json_config(&claude_settings, input.expected_hook_bin.as_deref(), health);
    }
    if codex_hooks.exists() {
        audit_hook_json_config(&codex_hooks, input.expected_hook_bin.as_deref(), health);
    }
}

fn audit_hook_json_config(
    path: &Path,
    expected_hook_bin: Option<&str>,
    health: &mut ManagedHookHealth,
) {
    let Ok(raw) = fs::read_to_string(path) else {
        degraded(
            health,
            format!("managed hook config could not be read: {}", path.display()),
        );
        return;
    };
    let Ok(root) = serde_json::from_str::<Value>(&raw) else {
        degraded(
            health,
            format!("managed hook config is not valid JSON: {}", path.display()),
        );
        return;
    };

    for event in MANAGED_EVENTS {
        let commands = hook_commands_for_event(&root, event);
        if !commands
            .iter()
            .any(|command| is_managed_event_command(command, event))
        {
            needs_attention(
                health,
                format!(
                    "managed hook event missing: {} in {}",
                    event,
                    path.display()
                ),
            );
        }
        if let Some(expected) = expected_hook_bin {
            for command in commands {
                if !is_managed_event_command(&command, event) {
                    continue;
                }
                let Some(actual) = hook_command_binary_prefix(&command) else {
                    continue;
                };
                if actual != expected {
                    degraded(
                        health,
                        format!(
                            "managed hook binary skew: {} uses {}, expected {}",
                            path.display(),
                            actual,
                            expected
                        ),
                    );
                }
            }
        }
    }
}

fn hook_commands_for_event(root: &Value, event: &str) -> Vec<String> {
    let Some(groups) = root
        .get("hooks")
        .and_then(Value::as_object)
        .and_then(|hooks| hooks.get(event))
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };
    groups
        .iter()
        .filter_map(|group| group.get("hooks").and_then(Value::as_array))
        .flat_map(|hooks| hooks.iter())
        .filter_map(|hook| hook.get("command").and_then(Value::as_str))
        .map(ToOwned::to_owned)
        .collect()
}

fn is_managed_event_command(command: &str, event: &str) -> bool {
    command.contains(&format!("hook event {event}"))
}

fn hook_command_binary_prefix(command: &str) -> Option<String> {
    let (prefix, _) = command.split_once(" hook ")?;
    let prefix = prefix
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string();
    (!prefix.is_empty()).then_some(prefix)
}

fn needs_attention(health: &mut ManagedHookHealth, issue: impl Into<String>) {
    if health.status == ManagedHookHealthStatus::Ready
        || health.status == ManagedHookHealthStatus::Inactive
        || health.status == ManagedHookHealthStatus::WaitingForFirstHookEvent
    {
        health.status = ManagedHookHealthStatus::NeedsAttention;
    }
    health.issues.push(issue.into());
}

fn degraded(health: &mut ManagedHookHealth, issue: impl Into<String>) {
    health.status = ManagedHookHealthStatus::Degraded;
    health.issues.push(issue.into());
}

fn read_runtime_state(path: &Path) -> Result<RuntimeStateReadModel, String> {
    let raw = fs::read_to_string(path).map_err(|error| error.to_string())?;
    serde_json::from_str(&raw).map_err(|error| error.to_string())
}

pub fn repair_managed_hook_configs(worktree_root: &Path) -> io::Result<ManagedHookRepairOutcome> {
    let claude_surface = worktree_root.join(".claude").exists()
        || worktree_root.join(".claude/settings.local.json").exists();
    let codex_surface =
        worktree_root.join(".codex").exists() || worktree_root.join(".codex/hooks.json").exists();
    let mut repaired = false;

    if claude_surface {
        gwt_skills::generate_settings_local(worktree_root)?;
        repaired = true;
    }
    if codex_surface {
        gwt_skills::generate_codex_hooks(worktree_root)?;
        repaired = true;
    }

    Ok(ManagedHookRepairOutcome { repaired })
}
