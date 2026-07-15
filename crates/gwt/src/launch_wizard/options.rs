use std::path::PathBuf;

use super::*;

#[derive(Clone, Copy)]
pub(super) struct ModelDisplayOption {
    pub(super) label: &'static str,
    pub(super) description: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct ReasoningDisplayOption {
    pub(super) label: &'static str,
    pub(super) stored_value: &'static str,
    pub(super) description: &'static str,
    pub(super) is_default: bool,
}

#[derive(Clone, Copy)]
pub(super) struct ChoiceOption {
    pub(super) label: &'static str,
    pub(super) description: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct ExecutionModeOption {
    pub(super) label: &'static str,
    pub(super) description: &'static str,
    pub(super) value: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct DockerLifecycleOption {
    pub(super) label: &'static str,
    pub(super) description: &'static str,
    pub(super) intent: gwt_agent::DockerLifecycleIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum QuickStartAction {
    ReuseEntry { index: usize },
    StartNewEntry { index: usize },
    FocusExistingSession,
    ChooseDifferent,
}

pub(super) fn default_launch_path(
    context: &LaunchWizardContext,
    quick_start_entries: &[QuickStartEntry],
) -> LaunchWizardLaunchPath {
    if !quick_start_entries.is_empty() {
        LaunchWizardLaunchPath::QuickStart
    } else if !context.live_sessions.is_empty() {
        LaunchWizardLaunchPath::FocusSession
    } else {
        LaunchWizardLaunchPath::ManualSetup
    }
}

const CLAUDE_DEFAULT_MODEL_LABEL: &str = "Default (Opus 4.8)";

// Fable 5 shares the Opus 4.7/4.8 effort surface (low..max), so both models
// use the same opus-tier reasoning ladder.
pub(super) fn is_claude_opus_tier_model(model: &str) -> bool {
    model == CLAUDE_DEFAULT_MODEL_LABEL || model == "opus" || model == "fable"
}

pub(super) fn is_claude_effort_capable_model(model: &str) -> bool {
    is_claude_opus_tier_model(model) || model == "sonnet"
}

const CLAUDE_MODEL_OPTIONS: [ModelDisplayOption; 5] = [
    ModelDisplayOption {
        label: CLAUDE_DEFAULT_MODEL_LABEL,
        description: "Most capable for complex work",
    },
    ModelDisplayOption {
        label: "fable",
        description: "Most capable for the hardest, longest-running tasks",
    },
    ModelDisplayOption {
        label: "opus",
        description: "Deep reasoning for complex problems",
    },
    ModelDisplayOption {
        label: "sonnet",
        description: "Balanced speed and capability",
    },
    ModelDisplayOption {
        label: "haiku",
        description: "Fastest option for light tasks",
    },
];

#[derive(Clone, Copy)]
pub(super) struct CodexModelCapability {
    pub(super) model: ModelDisplayOption,
    pub(super) default_effort: &'static str,
    pub(super) max_effort: &'static str,
}

// SPEC-1921 US-20 / FR-121..FR-123: fixed 2026-07-10 Codex picker snapshot.
// Model rows and reasoning rows both derive from this single capability table
// so stop counts and defaults cannot drift from the model list. A later
// snapshot update edits this table together with the focused tests; the
// wizard never reads a runtime model cache for these rows.
const CODEX_MODEL_CAPABILITIES: [CodexModelCapability; 7] = [
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.5",
            description: "Frontier model for complex coding, research, and real-world work",
        },
        default_effort: "medium",
        max_effort: "xhigh",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.6-sol",
            description: "Latest frontier agentic coding model",
        },
        default_effort: "low",
        max_effort: "ultra",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.6-terra",
            description: "Balanced agentic coding model for everyday work",
        },
        default_effort: "medium",
        max_effort: "ultra",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.6-luna",
            description: "Fast and affordable agentic coding model",
        },
        default_effort: "medium",
        max_effort: "max",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.4",
            description: "Strong model for everyday coding",
        },
        default_effort: "medium",
        max_effort: "xhigh",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.4-mini",
            description: "Small, fast, and cost-efficient model for simpler coding tasks",
        },
        default_effort: "medium",
        max_effort: "xhigh",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.3-codex-spark",
            description: "Ultra-fast coding model",
        },
        default_effort: "high",
        max_effort: "xhigh",
    },
];

const CODEX_MODEL_OPTIONS: [ModelDisplayOption; CODEX_MODEL_CAPABILITIES.len()] = {
    let mut options = [CODEX_MODEL_CAPABILITIES[0].model; CODEX_MODEL_CAPABILITIES.len()];
    let mut index = 0;
    while index < CODEX_MODEL_CAPABILITIES.len() {
        options[index] = CODEX_MODEL_CAPABILITIES[index].model;
        index += 1;
    }
    options
};

const GEMINI_MODEL_OPTIONS: [ModelDisplayOption; 7] = [
    ModelDisplayOption {
        label: "Default (Auto)",
        description: "Use Gemini default model",
    },
    ModelDisplayOption {
        label: "gemini-3-flash-preview",
        description: "Preview flash model",
    },
    ModelDisplayOption {
        label: "gemini-3.1-flash-lite-preview",
        description: "Preview flash-lite model",
    },
    ModelDisplayOption {
        label: "gemini-2.5-flash",
        description: "Stable flash model",
    },
    ModelDisplayOption {
        label: "gemini-2.5-flash-lite",
        description: "Stable flash-lite model",
    },
    ModelDisplayOption {
        label: "gemma-4-31b-it",
        description: "Gemma 4 31B instruction model",
    },
    ModelDisplayOption {
        label: "gemma-4-26b-a4b-it",
        description: "Gemma 4 26B A4B instruction model",
    },
];

// Auto is the default: gwt skips the CLAUDE_CODE_EFFORT_LEVEL export so
// Claude Code applies its own per-model default effort (`high` on
// Fable 5 / Opus 4.8, `xhigh` on Opus 4.7). Hardcoding a level here goes
// stale whenever a model generation changes its default, and the `opus`
// alias resolves to different generations per provider.
pub(super) const CLAUDE_OPUS_REASONING_OPTIONS: [ReasoningDisplayOption; 7] = [
    ReasoningDisplayOption {
        label: "Auto",
        stored_value: "auto",
        description: "Follow Claude Code's default effort for the model",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast responses for simple work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balanced reasoning for everyday work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Balances tokens and intelligence (Fable 5 / Opus 4.8 default)",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "xHigh",
        stored_value: "xhigh",
        description: "Deeper reasoning at higher token spend",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Max",
        stored_value: "max",
        description: "Deepest reasoning with no token-spending constraint",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Ultracode",
        stored_value: "ultracode",
        description: "Top-tier effort plus dynamic workflow orchestration (Opus-tier only)",
        is_default: false,
    },
];

pub(super) const CLAUDE_SONNET_REASONING_OPTIONS: [ReasoningDisplayOption; 4] = [
    ReasoningDisplayOption {
        label: "Auto",
        stored_value: "auto",
        description: "Follow Claude Code's default effort for the model",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast responses for simple work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balanced reasoning for everyday work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Deeper reasoning for complex work (Sonnet's default under Auto)",
        is_default: false,
    },
];

// Full Codex reasoning ladder in ascending depth order. Per-model rows take a
// prefix of this ladder up to the model's `max_effort` and mark the model's
// `default_effort` row; descriptions mirror the Codex CLI picker copy.
const CODEX_REASONING_LADDER: [ReasoningDisplayOption; 6] = [
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast responses with lighter reasoning",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balances speed and reasoning depth for everyday tasks",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Greater reasoning depth for complex problems",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Extra high",
        stored_value: "xhigh",
        description: "Extra high reasoning depth for complex problems",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Max",
        stored_value: "max",
        description: "Maximum reasoning depth for the hardest problems",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Ultra",
        stored_value: "ultra",
        description: "Maximum reasoning with automatic task delegation",
        is_default: false,
    },
];

// Unknown or legacy persisted Codex models keep the conservative pre-5.6
// surface so a stale saved model can never unlock stops the CLI would reject.
const CODEX_FALLBACK_DEFAULT_EFFORT: &str = "medium";
const CODEX_FALLBACK_MAX_EFFORT: &str = "xhigh";

pub(super) fn codex_reasoning_options_for_model(model: &str) -> Vec<ReasoningDisplayOption> {
    let capability = CODEX_MODEL_CAPABILITIES
        .iter()
        .find(|capability| capability.model.label == model);
    let default_effort = capability.map_or(CODEX_FALLBACK_DEFAULT_EFFORT, |row| row.default_effort);
    let max_effort = capability.map_or(CODEX_FALLBACK_MAX_EFFORT, |row| row.max_effort);
    let end = CODEX_REASONING_LADDER
        .iter()
        .position(|option| option.stored_value == max_effort)
        .map_or(CODEX_REASONING_LADDER.len(), |index| index + 1);
    CODEX_REASONING_LADDER[..end]
        .iter()
        .map(|option| ReasoningDisplayOption {
            is_default: option.stored_value == default_effort,
            ..*option
        })
        .collect()
}

pub(super) const EXECUTION_MODE_OPTIONS: [ExecutionModeOption; 3] = [
    ExecutionModeOption {
        label: "Normal",
        description: "Start a new session",
        value: "normal",
    },
    ExecutionModeOption {
        label: "Continue",
        description: "Continue from the last session",
        value: "continue",
    },
    ExecutionModeOption {
        label: "Resume",
        description: "Open the agent's session picker",
        value: "resume",
    },
];

pub(super) const RUNTIME_TARGET_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "Host",
        description: "Run directly on the host",
    },
    ChoiceOption {
        label: "Docker",
        description: "Run inside the detected Docker service",
    },
];

pub(super) const WINDOWS_SHELL_OPTIONS: [gwt_agent::WindowsShellKind; 3] = [
    gwt_agent::WindowsShellKind::CommandPrompt,
    gwt_agent::WindowsShellKind::WindowsPowerShell,
    gwt_agent::WindowsShellKind::PowerShell7,
];

pub(super) const YES_NO_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "Yes",
        description: "Skip permission prompts",
    },
    ChoiceOption {
        label: "No",
        description: "Show permission prompts",
    },
];

pub(super) const FAST_MODE_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "On",
        description: "Use the agent's Fast mode",
    },
    ChoiceOption {
        label: "Off",
        description: "Use the standard service tier",
    },
];

pub(super) fn default_docker_lifecycle_intent(
    status: gwt_docker::ComposeServiceStatus,
) -> gwt_agent::DockerLifecycleIntent {
    match status {
        gwt_docker::ComposeServiceStatus::Unknown => gwt_agent::DockerLifecycleIntent::Start,
        gwt_docker::ComposeServiceStatus::Running => gwt_agent::DockerLifecycleIntent::Connect,
        gwt_docker::ComposeServiceStatus::Stopped | gwt_docker::ComposeServiceStatus::Exited => {
            gwt_agent::DockerLifecycleIntent::Start
        }
        gwt_docker::ComposeServiceStatus::NotFound => {
            gwt_agent::DockerLifecycleIntent::CreateAndStart
        }
    }
}

/// SPEC-2014 FR-032..FR-035:
/// Launch Wizard 初期 `runtime_target` / `docker_service` /
/// `docker_lifecycle_intent` を、現在の Docker context と repo-local previous
/// profile から決定する。優先順は
/// `repo-local previous session` → `docker context default` で、open Wizard
/// draft は wizard 起動直後には存在しないので呼び出し側で考慮する必要は無い。
pub(super) fn resolve_initial_runtime_selection(
    context: &LaunchWizardContext,
    repo_local_previous: Option<&LaunchWizardPreviousProfile>,
) -> (
    gwt_agent::LaunchRuntimeTarget,
    Option<String>,
    gwt_agent::DockerLifecycleIntent,
) {
    // SPEC-2014 FR-013: 既存正規化チェーン (suggested -> first -> Host fallback)。
    // docker_context があっても services が空、または stale saved service で
    // 全ての候補が消えた場合は Host に落とす。
    let context_default_service = context.docker_context.as_ref().and_then(|ctx| {
        ctx.suggested_service
            .clone()
            .or_else(|| ctx.services.first().cloned())
    });
    let context_default_target = if context_default_service.is_some() {
        gwt_agent::LaunchRuntimeTarget::Docker
    } else {
        gwt_agent::LaunchRuntimeTarget::Host
    };
    let context_default_lifecycle = default_docker_lifecycle_intent(context.docker_service_status);

    let Some(saved) = repo_local_previous else {
        return (
            context_default_target,
            context_default_service,
            context_default_lifecycle,
        );
    };

    match saved.runtime_target {
        gwt_agent::LaunchRuntimeTarget::Host => {
            // FR-033: saved=Host は Docker context の有無に関わらず Host を維持し、
            // service/lifecycle UI も表示しない。
            (
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
                default_docker_lifecycle_intent(context.docker_service_status),
            )
        }
        gwt_agent::LaunchRuntimeTarget::Docker => match context.docker_context.as_ref() {
            // FR-034: saved=Docker かつ context 無し → Host に fallback。
            None => (
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
                default_docker_lifecycle_intent(context.docker_service_status),
            ),
            // FR-034: saved service が現在の services にあれば session の値を採用。
            // 無ければ既存 FR-013 の正規化 (suggested → first → 既定) を経由する。
            Some(docker_context) => {
                let saved_service_in_context = saved
                    .docker_service
                    .as_ref()
                    .filter(|name| docker_context.services.iter().any(|svc| svc == *name))
                    .cloned();
                if let Some(service) = saved_service_in_context {
                    (
                        gwt_agent::LaunchRuntimeTarget::Docker,
                        Some(service),
                        saved.docker_lifecycle_intent,
                    )
                } else {
                    (
                        gwt_agent::LaunchRuntimeTarget::Docker,
                        context_default_service,
                        context_default_lifecycle,
                    )
                }
            }
        },
    }
}

struct LaunchWizardFlow<'a> {
    state: &'a LaunchWizardState,
}

impl<'a> LaunchWizardFlow<'a> {
    fn new(state: &'a LaunchWizardState) -> Self {
        Self { state }
    }

    fn next_step(&self, current: LaunchWizardStep) -> Option<LaunchWizardStep> {
        match current {
            LaunchWizardStep::QuickStart => match self.state.selected_quick_start_action() {
                QuickStartAction::ChooseDifferent => Some(LaunchWizardStep::BranchAction),
                QuickStartAction::FocusExistingSession => {
                    Some(LaunchWizardStep::FocusExistingSession)
                }
                QuickStartAction::ReuseEntry { .. } | QuickStartAction::StartNewEntry { .. } => {
                    Some(LaunchWizardStep::SkipPermissions)
                }
            },
            LaunchWizardStep::FocusExistingSession => None,
            LaunchWizardStep::BranchAction => {
                if self.state.selected == 0 {
                    Some(LaunchWizardStep::LaunchTarget)
                } else {
                    Some(LaunchWizardStep::BranchTypeSelect)
                }
            }
            LaunchWizardStep::BranchTypeSelect => Some(LaunchWizardStep::BranchNameInput),
            LaunchWizardStep::BranchNameInput => Some(LaunchWizardStep::LaunchTarget),
            LaunchWizardStep::LaunchTarget => self.next_after_launch_target(),
            LaunchWizardStep::AgentSelect => {
                if self.state.agent_has_models() {
                    Some(LaunchWizardStep::ModelSelect)
                } else {
                    self.next_after_agent_configuration()
                }
            }
            LaunchWizardStep::ModelSelect => {
                if self.state.agent_uses_reasoning_step() {
                    Some(LaunchWizardStep::ReasoningLevel)
                } else {
                    self.next_after_agent_configuration()
                }
            }
            LaunchWizardStep::ReasoningLevel => self.next_after_agent_configuration(),
            LaunchWizardStep::RuntimeTarget => self.next_after_runtime_target(),
            LaunchWizardStep::WindowsShell => self.next_after_windows_shell(),
            LaunchWizardStep::DockerServiceSelect => Some(LaunchWizardStep::DockerLifecycle),
            LaunchWizardStep::DockerLifecycle => self.next_after_docker_lifecycle(),
            LaunchWizardStep::VersionSelect => Some(LaunchWizardStep::SkipPermissions),
            LaunchWizardStep::ExecutionMode => Some(LaunchWizardStep::SkipPermissions),
            LaunchWizardStep::SkipPermissions => {
                if self.state.current_agent_supports_fast_mode() {
                    Some(LaunchWizardStep::CodexFastMode)
                } else {
                    None
                }
            }
            LaunchWizardStep::CodexFastMode => None,
        }
    }

    fn prev_step(&self, current: LaunchWizardStep) -> Option<LaunchWizardStep> {
        match current {
            LaunchWizardStep::QuickStart => None,
            LaunchWizardStep::FocusExistingSession => Some(LaunchWizardStep::QuickStart),
            LaunchWizardStep::BranchAction => {
                if !self.state.quick_start_entries.is_empty()
                    || !self.state.context.live_sessions.is_empty()
                {
                    Some(LaunchWizardStep::QuickStart)
                } else {
                    None
                }
            }
            LaunchWizardStep::BranchTypeSelect => Some(LaunchWizardStep::BranchAction),
            LaunchWizardStep::BranchNameInput => Some(LaunchWizardStep::BranchTypeSelect),
            LaunchWizardStep::LaunchTarget => {
                if self.state.is_new_branch {
                    Some(LaunchWizardStep::BranchNameInput)
                } else {
                    Some(LaunchWizardStep::BranchAction)
                }
            }
            LaunchWizardStep::AgentSelect => Some(LaunchWizardStep::LaunchTarget),
            LaunchWizardStep::ModelSelect => Some(LaunchWizardStep::AgentSelect),
            LaunchWizardStep::ReasoningLevel => Some(LaunchWizardStep::ModelSelect),
            LaunchWizardStep::RuntimeTarget => {
                if self.state.launch_target_is_shell() {
                    Some(LaunchWizardStep::LaunchTarget)
                } else {
                    self.previous_agent_configuration_step()
                }
            }
            LaunchWizardStep::WindowsShell => self.previous_before_windows_shell(),
            LaunchWizardStep::DockerServiceSelect => Some(LaunchWizardStep::RuntimeTarget),
            LaunchWizardStep::DockerLifecycle => {
                if self.state.docker_service_prompt_required() {
                    Some(LaunchWizardStep::DockerServiceSelect)
                } else {
                    Some(LaunchWizardStep::RuntimeTarget)
                }
            }
            LaunchWizardStep::VersionSelect => self.previous_before_version_select(),
            LaunchWizardStep::ExecutionMode => self.previous_before_execution_mode(),
            LaunchWizardStep::SkipPermissions => self.previous_before_execution_mode(),
            LaunchWizardStep::CodexFastMode => Some(LaunchWizardStep::SkipPermissions),
        }
    }

    fn next_after_launch_target(&self) -> Option<LaunchWizardStep> {
        if self.state.launch_target_is_agent() {
            Some(LaunchWizardStep::AgentSelect)
        } else if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else {
            self.next_after_host_runtime()
        }
    }

    fn next_after_agent_configuration(&self) -> Option<LaunchWizardStep> {
        if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else {
            self.next_after_host_runtime()
        }
    }

    fn next_after_runtime_target(&self) -> Option<LaunchWizardStep> {
        if self.state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker
            && self.state.docker_service_prompt_required()
        {
            Some(LaunchWizardStep::DockerServiceSelect)
        } else if self.state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
            Some(LaunchWizardStep::DockerLifecycle)
        } else {
            self.next_after_host_runtime()
        }
    }

    fn next_after_host_runtime(&self) -> Option<LaunchWizardStep> {
        if self.state.runtime_context_resolved && self.state.show_windows_shell_selection() {
            Some(LaunchWizardStep::WindowsShell)
        } else {
            self.next_after_windows_shell()
        }
    }

    fn next_after_windows_shell(&self) -> Option<LaunchWizardStep> {
        if self.state.launch_target_is_shell() {
            None
        } else if agent_has_npm_package(self.state.effective_agent_id()) {
            Some(LaunchWizardStep::VersionSelect)
        } else {
            Some(LaunchWizardStep::SkipPermissions)
        }
    }

    fn next_after_docker_lifecycle(&self) -> Option<LaunchWizardStep> {
        if self.state.launch_target_is_shell() {
            None
        } else if agent_has_npm_package(self.state.effective_agent_id()) {
            Some(LaunchWizardStep::VersionSelect)
        } else {
            Some(LaunchWizardStep::SkipPermissions)
        }
    }

    fn previous_agent_configuration_step(&self) -> Option<LaunchWizardStep> {
        if self.state.agent_uses_reasoning_step() {
            Some(LaunchWizardStep::ReasoningLevel)
        } else if self.state.agent_has_models() {
            Some(LaunchWizardStep::ModelSelect)
        } else {
            Some(LaunchWizardStep::AgentSelect)
        }
    }

    fn previous_before_windows_shell(&self) -> Option<LaunchWizardStep> {
        if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else if self.state.launch_target_is_shell() {
            Some(LaunchWizardStep::LaunchTarget)
        } else {
            self.previous_agent_configuration_step()
        }
    }

    fn previous_before_version_select(&self) -> Option<LaunchWizardStep> {
        if self.state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
            Some(LaunchWizardStep::DockerLifecycle)
        } else if self.state.runtime_context_resolved && self.state.show_windows_shell_selection() {
            Some(LaunchWizardStep::WindowsShell)
        } else if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else {
            self.previous_agent_configuration_step()
        }
    }

    fn previous_before_execution_mode(&self) -> Option<LaunchWizardStep> {
        if self.state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
            Some(LaunchWizardStep::DockerLifecycle)
        } else if self.state.runtime_context_resolved && self.state.show_windows_shell_selection() {
            Some(LaunchWizardStep::WindowsShell)
        } else if agent_has_npm_package(self.state.effective_agent_id()) {
            Some(LaunchWizardStep::VersionSelect)
        } else if self.state.has_docker_workflow() {
            Some(LaunchWizardStep::RuntimeTarget)
        } else {
            self.previous_agent_configuration_step()
        }
    }
}

pub(super) fn next_step(
    current: LaunchWizardStep,
    state: &LaunchWizardState,
) -> Option<LaunchWizardStep> {
    LaunchWizardFlow::new(state).next_step(current)
}

pub(super) fn prev_step(
    current: LaunchWizardStep,
    state: &LaunchWizardState,
) -> Option<LaunchWizardStep> {
    LaunchWizardFlow::new(state).prev_step(current)
}

pub(super) fn step_default_selection(step: LaunchWizardStep, state: &LaunchWizardState) -> usize {
    match step {
        LaunchWizardStep::QuickStart => 0,
        LaunchWizardStep::FocusExistingSession => 0,
        LaunchWizardStep::BranchAction => 0,
        LaunchWizardStep::BranchTypeSelect => 0,
        LaunchWizardStep::BranchNameInput => 0,
        LaunchWizardStep::LaunchTarget => usize::from(state.launch_target_is_shell()),
        LaunchWizardStep::AgentSelect => state
            .detected_agents
            .iter()
            .position(|agent| agent.id == state.agent_id)
            .unwrap_or(0),
        LaunchWizardStep::ModelSelect => current_model_options(state.effective_agent_id())
            .iter()
            .position(|model| model == &state.model)
            .unwrap_or(0),
        LaunchWizardStep::ReasoningLevel => state
            .current_reasoning_options()
            .iter()
            .position(|option| option.stored_value == state.reasoning)
            .unwrap_or_else(|| {
                state
                    .current_reasoning_options()
                    .iter()
                    .position(|option| option.is_default)
                    .unwrap_or(0)
            }),
        LaunchWizardStep::RuntimeTarget => {
            usize::from(state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker)
        }
        LaunchWizardStep::WindowsShell => WINDOWS_SHELL_OPTIONS
            .iter()
            .position(|option| *option == state.windows_shell)
            .unwrap_or(0),
        LaunchWizardStep::DockerServiceSelect => state
            .preferred_docker_service()
            .and_then(|service| {
                state
                    .docker_service_options()
                    .iter()
                    .position(|option| option == service)
            })
            .unwrap_or(0),
        LaunchWizardStep::DockerLifecycle => state
            .docker_lifecycle_options()
            .iter()
            .position(|option| option.intent == state.docker_lifecycle_intent)
            .unwrap_or(0),
        LaunchWizardStep::VersionSelect => state
            .current_version_options()
            .iter()
            .position(|option| option.value == state.version)
            .unwrap_or(0),
        LaunchWizardStep::ExecutionMode => state
            .execution_mode_step_options()
            .iter()
            .position(|option| option.value == state.mode)
            .unwrap_or(0),
        LaunchWizardStep::SkipPermissions => usize::from(!state.skip_permissions),
        LaunchWizardStep::CodexFastMode => {
            usize::from(!state.fast_mode_enabled_for_current_agent())
        }
    }
}

pub(super) fn current_model_options(agent_id: &str) -> Vec<&'static str> {
    match agent_id {
        "claude" => CLAUDE_MODEL_OPTIONS
            .iter()
            .map(|option| option.label)
            .collect(),
        "codex" => CODEX_MODEL_OPTIONS
            .iter()
            .map(|option| option.label)
            .collect(),
        "gemini" => GEMINI_MODEL_OPTIONS
            .iter()
            .map(|option| option.label)
            .collect(),
        _ => Vec::new(),
    }
}

pub(super) fn model_display_options(agent_id: &str) -> &'static [ModelDisplayOption] {
    match agent_id {
        "claude" => &CLAUDE_MODEL_OPTIONS,
        "codex" => &CODEX_MODEL_OPTIONS,
        "gemini" => &GEMINI_MODEL_OPTIONS,
        _ => &[],
    }
}

pub(super) fn quick_start_summary(entry: &QuickStartEntry) -> String {
    let mut parts = vec![entry.tool_label.clone()];
    if let Some(model) = entry.model.as_deref() {
        parts.push(model.to_string());
    }
    if let Some(reasoning) = entry.reasoning.as_deref() {
        parts.push(reasoning.to_string());
    }
    if let Some(version) = entry.version.as_deref() {
        parts.push(version.to_string());
    }
    if entry.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
        parts.push(
            entry
                .docker_service
                .as_ref()
                .map(|service| format!("docker:{service}"))
                .unwrap_or_else(|| "docker".to_string()),
        );
    }
    parts.join(" · ")
}

pub(super) fn branch_type_options_view() -> Vec<LaunchWizardOptionView> {
    BRANCH_TYPE_PREFIXES
        .iter()
        .map(|prefix| LaunchWizardOptionView {
            value: (*prefix).to_string(),
            label: (*prefix).to_string(),
            description: Some(format!(
                "Use {} as the branch prefix",
                prefix.trim_end_matches('/')
            )),
            color: None,
        })
        .collect()
}

pub(super) fn launch_target_options_view() -> Vec<LaunchWizardOptionView> {
    vec![
        LaunchWizardOptionView {
            value: "agent".to_string(),
            label: "Agent".to_string(),
            description: Some("Launch a coding agent terminal".to_string()),
            color: None,
        },
        LaunchWizardOptionView {
            value: "shell".to_string(),
            label: "Shell".to_string(),
            description: Some("Open a plain shell terminal".to_string()),
            color: None,
        },
    ]
}

pub(super) fn runtime_target_options_view() -> Vec<LaunchWizardOptionView> {
    RUNTIME_TARGET_OPTIONS
        .iter()
        .map(|option| LaunchWizardOptionView {
            value: option.label.to_ascii_lowercase(),
            label: option.label.to_string(),
            description: Some(option.description.to_string()),
            color: None,
        })
        .collect()
}

pub(super) fn windows_shell_options_view() -> Vec<LaunchWizardOptionView> {
    WINDOWS_SHELL_OPTIONS
        .iter()
        .copied()
        .map(|shell| LaunchWizardOptionView {
            value: windows_shell_option_value(shell).to_string(),
            label: windows_shell_option_label(shell).to_string(),
            description: Some(windows_shell_option_description(shell).to_string()),
            color: None,
        })
        .collect()
}

pub(super) fn windows_shell_option_value(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "command_prompt",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "windows_power_shell",
        gwt_agent::WindowsShellKind::PowerShell7 => "power_shell_7",
    }
}

pub(super) fn windows_shell_option_label(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "Command Prompt",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "Windows PowerShell",
        gwt_agent::WindowsShellKind::PowerShell7 => "PowerShell 7",
    }
}

pub(super) fn windows_shell_option_description(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "Run through cmd.exe",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "Run through Windows PowerShell",
        gwt_agent::WindowsShellKind::PowerShell7 => "Run through PowerShell 7",
    }
}

fn windows_shell_detection_command(shell: gwt_agent::WindowsShellKind) -> &'static str {
    match shell {
        gwt_agent::WindowsShellKind::CommandPrompt => "cmd.exe",
        gwt_agent::WindowsShellKind::WindowsPowerShell => "powershell",
        gwt_agent::WindowsShellKind::PowerShell7 => "pwsh",
    }
}

pub(super) fn default_windows_shell_kind() -> gwt_agent::WindowsShellKind {
    default_windows_shell_kind_with(gwt_core::process::command_exists)
}

pub(super) fn default_windows_shell_kind_with<F>(
    mut command_exists: F,
) -> gwt_agent::WindowsShellKind
where
    F: FnMut(&str) -> bool,
{
    if command_exists(windows_shell_detection_command(
        gwt_agent::WindowsShellKind::PowerShell7,
    )) {
        return gwt_agent::WindowsShellKind::PowerShell7;
    }
    if command_exists(windows_shell_detection_command(
        gwt_agent::WindowsShellKind::WindowsPowerShell,
    )) {
        return gwt_agent::WindowsShellKind::WindowsPowerShell;
    }
    gwt_agent::WindowsShellKind::CommandPrompt
}

pub(super) fn execution_mode_options_view(
    supports_resume_picker: bool,
) -> Vec<LaunchWizardOptionView> {
    EXECUTION_MODE_OPTIONS
        .iter()
        .filter(|option| supports_resume_picker || option.value != "resume")
        .map(|option| LaunchWizardOptionView {
            value: option.value.to_string(),
            label: option.label.to_string(),
            description: Some(option.description.to_string()),
            color: None,
        })
        .collect()
}

pub(super) fn execution_mode_value_from_session_mode(mode: gwt_agent::SessionMode) -> &'static str {
    match mode {
        gwt_agent::SessionMode::Normal => "normal",
        gwt_agent::SessionMode::Continue => "continue",
        gwt_agent::SessionMode::Resume => "resume",
    }
}

pub(super) fn launch_target_value(target: LaunchTargetKind) -> &'static str {
    match target {
        LaunchTargetKind::Agent => "agent",
        LaunchTargetKind::Shell => "shell",
    }
}

pub(super) fn runtime_target_value(target: gwt_agent::LaunchRuntimeTarget) -> &'static str {
    match target {
        gwt_agent::LaunchRuntimeTarget::Host => "host",
        gwt_agent::LaunchRuntimeTarget::Docker => "docker",
    }
}

pub(super) fn window_status_wire(status: crate::WindowProcessStatus) -> &'static str {
    match status {
        crate::WindowProcessStatus::Running => "running",
        crate::WindowProcessStatus::Starting => "starting",
        crate::WindowProcessStatus::Idle => "idle",
        crate::WindowProcessStatus::Waiting => "waiting",
        crate::WindowProcessStatus::Stopped => "stopped",
        crate::WindowProcessStatus::Error => "error",
    }
}

pub(super) fn live_session_status_label(session: &LiveSessionEntry) -> String {
    format!("Status · {}", window_status_wire(session.runtime_status))
}

pub(super) fn docker_lifecycle_value(intent: gwt_agent::DockerLifecycleIntent) -> &'static str {
    match intent {
        gwt_agent::DockerLifecycleIntent::Connect => "connect",
        gwt_agent::DockerLifecycleIntent::Start => "start",
        gwt_agent::DockerLifecycleIntent::Restart => "restart",
        gwt_agent::DockerLifecycleIntent::Recreate => "recreate",
        gwt_agent::DockerLifecycleIntent::CreateAndStart => "create_and_start",
    }
}

pub(super) fn is_explicit_model_selection(model: &str) -> bool {
    !model.is_empty() && !model.starts_with("Default")
}

pub(super) fn agent_has_npm_package(agent_id: &str) -> bool {
    agent_id_from_key(agent_id).package_name().is_some()
}

pub(super) fn agent_id_from_key(agent_id: &str) -> gwt_agent::AgentId {
    gwt_agent::builtin_agent_descriptor_for_command(agent_id)
        .map(|descriptor| descriptor.id.clone())
        .unwrap_or_else(|| gwt_agent::AgentId::Custom(agent_id.to_string()))
}

pub(super) fn agent_description(agent: &AgentOption) -> String {
    match agent.installed_version.as_deref() {
        Some(version) => format!("Detected · {version}"),
        None if agent.custom_agent.is_some() => "Configured".to_string(),
        None => "Built-in".to_string(),
    }
}

fn load_global_custom_agents() -> Vec<gwt_agent::CustomCodingAgent> {
    if std::env::var_os(gwt_agent::DISABLE_GLOBAL_CUSTOM_AGENTS_ENV).is_some() {
        return Vec::new();
    }

    gwt_agent::load_custom_agents_from_path(&gwt_core::paths::gwt_config_path()).unwrap_or_default()
}

/// Map the raw agent option id (command name or custom agent id) to the
/// AgentColor rendered on the Launch Wizard candidate row.
/// SPEC #2133 FR-009 / シナリオ 2.
pub(super) fn agent_option_color(agent_id: &str) -> Option<gwt_agent::AgentColor> {
    gwt_agent::resolve_agent_id(agent_id).map(|id| id.default_color())
}

pub fn default_wizard_version_cache_path() -> PathBuf {
    gwt_core::paths::gwt_cache_dir().join("agent-versions.json")
}

pub fn build_agent_options(
    detected_agents: Vec<gwt_agent::DetectedAgent>,
    cache: &gwt_agent::VersionCache,
    custom_agents: Vec<gwt_agent::CustomCodingAgent>,
) -> Vec<AgentOption> {
    let mut options = build_builtin_agent_options(detected_agents, cache);
    options.extend(custom_agents.into_iter().map(|agent| AgentOption {
        id: agent.id.clone(),
        name: agent.display_name.clone(),
        available: true,
        installed_version: None,
        versions: Vec::new(),
        custom_agent: Some(agent),
    }));
    options
}

pub fn load_agent_options(cache: &gwt_agent::VersionCache) -> Vec<AgentOption> {
    build_agent_options(Vec::new(), cache, load_global_custom_agents())
}

pub fn build_builtin_agent_options(
    detected_agents: Vec<gwt_agent::DetectedAgent>,
    cache: &gwt_agent::VersionCache,
) -> Vec<AgentOption> {
    gwt_agent::builtin_agent_descriptors()
        .iter()
        .map(|descriptor| {
            let agent_id = descriptor.id.clone();
            let detected = detected_agents
                .iter()
                .find(|detected| detected.agent_id == agent_id);
            AgentOption {
                id: agent_id.command().to_string(),
                name: agent_id.display_name().to_string(),
                available: true,
                installed_version: detected.and_then(|detected| detected.version.clone()),
                versions: cache
                    .get(&agent_id)
                    .map(<[std::string::String]>::to_vec)
                    .unwrap_or_default(),
                custom_agent: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::super::test_support::*;
    use super::*;

    #[test]
    fn agent_option_color_maps_known_ids_and_falls_back_to_gray() {
        assert_eq!(
            agent_option_color("claude"),
            Some(gwt_agent::AgentColor::Yellow)
        );
        assert_eq!(
            agent_option_color("codex"),
            Some(gwt_agent::AgentColor::Cyan)
        );
        assert_eq!(
            agent_option_color("gemini"),
            Some(gwt_agent::AgentColor::Magenta)
        );
        assert_eq!(
            agent_option_color("opencode"),
            Some(gwt_agent::AgentColor::Green)
        );
        assert_eq!(
            agent_option_color("openclaw"),
            Some(gwt_agent::AgentColor::Blue)
        );
        assert_eq!(
            agent_option_color("hermes"),
            Some(gwt_agent::AgentColor::Magenta)
        );
        assert_eq!(agent_option_color("gh"), Some(gwt_agent::AgentColor::Blue));
        assert_eq!(
            agent_option_color("my-custom"),
            Some(gwt_agent::AgentColor::Gray)
        );
        assert_eq!(agent_option_color(""), None);
    }

    #[test]
    fn build_agent_options_appends_config_backed_custom_agents_after_builtins() {
        let dir = tempdir().expect("tempdir");
        let available_path = dir.path().join("custom-agent");
        std::fs::write(&available_path, "echo custom").expect("write custom agent stub");
        let missing_path = dir.path().join("missing-agent");

        let options = build_agent_options(
            vec![gwt_agent::DetectedAgent {
                agent_id: gwt_agent::AgentId::ClaudeCode,
                version: Some("1.2.3".to_string()),
                path: PathBuf::from("/tmp/claude"),
            }],
            &gwt_agent::VersionCache::new(),
            vec![
                sample_custom_agent(
                    "proxy-agent",
                    "Claude Proxy",
                    gwt_agent::custom::CustomAgentType::Path,
                    available_path.display().to_string(),
                ),
                sample_custom_agent(
                    "missing-agent",
                    "Missing Agent",
                    gwt_agent::custom::CustomAgentType::Path,
                    missing_path.display().to_string(),
                ),
            ],
        );

        let proxy = options
            .iter()
            .position(|option| option.id == "proxy-agent")
            .expect("custom agent appended");
        let missing = options
            .iter()
            .position(|option| option.id == "missing-agent")
            .expect("missing custom agent appended");

        assert!(proxy > 0, "custom agents must appear after builtin options");
        assert!(missing > proxy, "custom agents should keep append order");
        assert_eq!(options[proxy].name, "Claude Proxy");
        assert!(options[proxy].available);
        assert!(
            options[missing].available,
            "configured custom agents must stay selectable; runtime preparation validates execution"
        );
    }

    #[test]
    fn build_builtin_agent_options_includes_hook_parity_agents() {
        let options = build_builtin_agent_options(Vec::new(), &gwt_agent::VersionCache::new());
        let ids: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();

        assert_eq!(
            ids,
            vec!["claude", "codex", "agy", "gemini", "opencode", "openclaw", "hermes", "gh"]
        );
        assert!(options
            .iter()
            .any(|option| option.name == "Antigravity CLI"));
        assert!(options
            .iter()
            .any(|option| option.name == "Gemini CLI (legacy)"));
        assert!(options.iter().any(|option| option.name == "OpenCode"));
        assert!(options.iter().any(|option| option.name == "OpenClaw"));
        assert!(options.iter().any(|option| option.name == "Hermes Agent"));
    }

    // SPEC-2014 2026-05-18 amendment FR-D / SC-C:
    // execution_mode_options_view filters `resume` for picker-unsupported
    // agents. The Launch Wizard view must match.
    #[test]
    fn execution_mode_options_omit_resume_for_picker_unsupported_agent() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        state.agent_id = "gemini".to_string();

        let view = state.view();
        assert!(
            view.execution_mode_options
                .iter()
                .all(|option| option.value != "resume"),
            "Gemini must not advertise the picker option: {:?}",
            view.execution_mode_options
        );

        state.agent_id = "claude".to_string();
        let view = state.view();
        assert!(view
            .execution_mode_options
            .iter()
            .any(|option| option.value == "resume"));

        state.agent_id = "codex".to_string();
        let view = state.view();
        assert!(view
            .execution_mode_options
            .iter()
            .any(|option| option.value == "resume"));
    }

    // SPEC-2014 2026-05-18 amendment FR-F / SC-E:
    // execution_mode_value_from_session_mode roundtrips Resume → "resume"
    // instead of collapsing to "continue", so previous-profile Resume can be
    // restored as picker mode (id intentionally cleared on restore).
    #[test]
    fn execution_mode_value_from_session_mode_round_trips_resume() {
        assert_eq!(
            execution_mode_value_from_session_mode(gwt_agent::SessionMode::Normal),
            "normal"
        );
        assert_eq!(
            execution_mode_value_from_session_mode(gwt_agent::SessionMode::Continue),
            "continue"
        );
        assert_eq!(
            execution_mode_value_from_session_mode(gwt_agent::SessionMode::Resume),
            "resume"
        );
    }

    #[test]
    fn default_windows_shell_kind_prefers_pwsh_then_windows_powershell_then_cmd() {
        let shell = default_windows_shell_kind_with(|command| command == "pwsh");
        assert_eq!(shell, gwt_agent::WindowsShellKind::PowerShell7);

        let shell = default_windows_shell_kind_with(|command| command == "powershell");
        assert_eq!(shell, gwt_agent::WindowsShellKind::WindowsPowerShell);

        let shell = default_windows_shell_kind_with(|_| false);
        assert_eq!(shell, gwt_agent::WindowsShellKind::CommandPrompt);
    }

    #[test]
    fn windows_shell_option_metadata_is_owned_by_launch_wizard() {
        assert_eq!(
            windows_shell_option_value(gwt_agent::WindowsShellKind::CommandPrompt),
            "command_prompt"
        );
        assert_eq!(
            windows_shell_option_label(gwt_agent::WindowsShellKind::WindowsPowerShell),
            "Windows PowerShell"
        );
        assert_eq!(
            windows_shell_option_description(gwt_agent::WindowsShellKind::PowerShell7),
            "Run through PowerShell 7"
        );
    }

    #[test]
    fn launch_wizard_flow_policy_centralizes_host_shell_step() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        let flow = LaunchWizardFlow::new(&state);
        let expected_host_tail = if cfg!(windows) {
            Some(LaunchWizardStep::WindowsShell)
        } else if agent_has_npm_package(state.effective_agent_id()) {
            Some(LaunchWizardStep::VersionSelect)
        } else {
            Some(LaunchWizardStep::SkipPermissions)
        };

        assert_eq!(flow.next_after_agent_configuration(), expected_host_tail);

        let mut docker = state.clone();
        docker.context.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("api".to_string()),
        });
        docker.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;

        assert_ne!(
            LaunchWizardFlow::new(&docker).next_after_runtime_target(),
            Some(LaunchWizardStep::WindowsShell)
        );
    }

    #[test]
    fn helper_value_functions_cover_docker_and_agent_variants() {
        assert_eq!(
            default_docker_lifecycle_intent(gwt_docker::ComposeServiceStatus::Running),
            gwt_agent::DockerLifecycleIntent::Connect
        );
        assert_eq!(
            default_docker_lifecycle_intent(gwt_docker::ComposeServiceStatus::Stopped),
            gwt_agent::DockerLifecycleIntent::Start
        );
        assert_eq!(
            default_docker_lifecycle_intent(gwt_docker::ComposeServiceStatus::NotFound),
            gwt_agent::DockerLifecycleIntent::CreateAndStart
        );
        assert_eq!(launch_target_value(LaunchTargetKind::Agent), "agent");
        assert_eq!(launch_target_value(LaunchTargetKind::Shell), "shell");
        assert_eq!(
            runtime_target_value(gwt_agent::LaunchRuntimeTarget::Host),
            "host"
        );
        assert_eq!(
            runtime_target_value(gwt_agent::LaunchRuntimeTarget::Docker),
            "docker"
        );
        assert_eq!(
            docker_lifecycle_value(gwt_agent::DockerLifecycleIntent::Restart),
            "restart"
        );
        assert_eq!(
            docker_lifecycle_value(gwt_agent::DockerLifecycleIntent::CreateAndStart),
            "create_and_start"
        );
        assert!(is_explicit_model_selection("gpt-5.5"));
        assert!(!is_explicit_model_selection("Default (Installed)"));
        assert!(agent_has_npm_package("codex"));
        assert!(agent_has_npm_package("opencode"));
        assert!(!agent_has_npm_package("openclaw"));
        assert!(!agent_has_npm_package("hermes"));
        assert!(!agent_has_npm_package("custom"));
        assert_eq!(agent_id_from_key("gh"), gwt_agent::AgentId::Copilot);
        assert_eq!(agent_id_from_key("opencode"), gwt_agent::AgentId::OpenCode);
        assert_eq!(agent_id_from_key("openclaw"), gwt_agent::AgentId::OpenClaw);
        assert_eq!(agent_id_from_key("hermes"), gwt_agent::AgentId::Hermes);
        assert_eq!(
            agent_id_from_key("custom"),
            gwt_agent::AgentId::Custom("custom".to_string())
        );
        assert_eq!(
            agent_description(&sample_agent_options()[0]),
            "Detected · 1.0.0".to_string()
        );
    }

    #[test]
    fn option_views_and_model_catalogs_expose_expected_labels() {
        let branch_types = branch_type_options_view();
        assert!(branch_types.iter().any(|option| option.value == "feature/"));
        assert!(branch_types
            .iter()
            .all(|option| option.description.as_deref().is_some()));

        let launch_targets = launch_target_options_view();
        assert_eq!(launch_targets[0].value, "agent");
        assert_eq!(launch_targets[1].value, "shell");

        let runtime_targets = runtime_target_options_view();
        assert!(runtime_targets.iter().any(|option| option.value == "host"));
        assert!(runtime_targets
            .iter()
            .any(|option| option.value == "docker"));

        let execution_modes = execution_mode_options_view(true);
        assert!(execution_modes
            .iter()
            .any(|option| option.value == "normal"));
        assert!(execution_modes
            .iter()
            .any(|option| option.value == "resume"));

        // SPEC-2014 2026-05-18 amendment FR-D / SC-C:
        // picker 非対応 capability では "resume" option を除外する。
        let modes_without_picker = execution_mode_options_view(false);
        assert!(modes_without_picker
            .iter()
            .all(|option| option.value != "resume"));
        assert!(modes_without_picker
            .iter()
            .any(|option| option.value == "normal"));
        assert!(modes_without_picker
            .iter()
            .any(|option| option.value == "continue"));

        assert_eq!(
            current_model_options("claude"),
            vec!["Default (Opus 4.8)", "fable", "opus", "sonnet", "haiku"]
        );
        assert_eq!(
            current_model_options("codex"),
            vec![
                "gpt-5.5",
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.4",
                "gpt-5.4-mini",
                "gpt-5.3-codex-spark",
            ]
        );
        assert_eq!(
            current_model_options("gemini"),
            vec![
                "Default (Auto)",
                "gemini-3-flash-preview",
                "gemini-3.1-flash-lite-preview",
                "gemini-2.5-flash",
                "gemini-2.5-flash-lite",
                "gemma-4-31b-it",
                "gemma-4-26b-a4b-it",
            ]
        );
        assert!(current_model_options("agy").is_empty());
        assert!(model_display_options("agy").is_empty());
        assert!(current_model_options("custom").is_empty());
        assert!(model_display_options("custom").is_empty());
        assert!(!model_display_options("codex").is_empty());
    }

    // SPEC-1921 US-20 / FR-121: the Codex picker is the fixed, tested
    // 2026-07-10 seven-model snapshot with the current descriptions.
    #[test]
    fn codex_model_catalog_matches_2026_07_10_snapshot() {
        let rows: Vec<(&str, &str)> = model_display_options("codex")
            .iter()
            .map(|option| (option.label, option.description))
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "gpt-5.5",
                    "Frontier model for complex coding, research, and real-world work",
                ),
                ("gpt-5.6-sol", "Latest frontier agentic coding model"),
                (
                    "gpt-5.6-terra",
                    "Balanced agentic coding model for everyday work",
                ),
                ("gpt-5.6-luna", "Fast and affordable agentic coding model"),
                ("gpt-5.4", "Strong model for everyday coding"),
                (
                    "gpt-5.4-mini",
                    "Small, fast, and cost-efficient model for simpler coding tasks",
                ),
                ("gpt-5.3-codex-spark", "Ultra-fast coding model"),
            ]
        );
    }

    fn codex_capability_row(model: &str) -> (Vec<&'static str>, &'static str) {
        let options = codex_reasoning_options_for_model(model);
        let values: Vec<&'static str> = options.iter().map(|option| option.stored_value).collect();
        let default = options
            .iter()
            .find(|option| option.is_default)
            .expect("codex reasoning rows must include a default stop")
            .stored_value;
        (values, default)
    }

    // SPEC-1921 US-20 / FR-122 + FR-123: reasoning rows and the initial stop
    // derive from the selected model's capability row, so Sol/Terra expose six
    // stops through Ultra, Luna five through Max, and the rest four through
    // Extra high, with Sol=Low / Spark=High / others=Medium defaults.
    #[test]
    fn codex_reasoning_capability_rows_follow_model() {
        const SIX: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];
        const FIVE: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
        const FOUR: [&str; 4] = ["low", "medium", "high", "xhigh"];

        assert_eq!(codex_capability_row("gpt-5.6-sol"), (SIX.to_vec(), "low"));
        assert_eq!(
            codex_capability_row("gpt-5.6-terra"),
            (SIX.to_vec(), "medium")
        );
        assert_eq!(
            codex_capability_row("gpt-5.6-luna"),
            (FIVE.to_vec(), "medium")
        );
        assert_eq!(codex_capability_row("gpt-5.5"), (FOUR.to_vec(), "medium"));
        assert_eq!(codex_capability_row("gpt-5.4"), (FOUR.to_vec(), "medium"));
        assert_eq!(
            codex_capability_row("gpt-5.4-mini"),
            (FOUR.to_vec(), "medium")
        );
        assert_eq!(
            codex_capability_row("gpt-5.3-codex-spark"),
            (FOUR.to_vec(), "high")
        );
    }

    // Unknown or legacy persisted Codex models keep the conservative pre-5.6
    // surface so a stale saved model can never unlock unsupported stops.
    #[test]
    fn codex_reasoning_capability_falls_back_conservatively_for_unknown_model() {
        let (values, default) = codex_capability_row("gpt-5.2-codex");
        assert_eq!(values, vec!["low", "medium", "high", "xhigh"]);
        assert_eq!(default, "medium");
    }

    #[test]
    fn quick_start_summary_includes_runtime_metadata() {
        let summary = quick_start_summary(&QuickStartEntry {
            session_id: "gwt-session-1".to_string(),
            agent_id: "codex".to_string(),
            tool_label: "Codex".to_string(),
            model: Some("gpt-5.5".to_string()),
            reasoning: Some("high".to_string()),
            version: Some("0.110.0".to_string()),
            resume_session_id: Some("resume-1".to_string()),
            live_window_id: None,
            skip_permissions: true,
            codex_fast_mode: true,
            runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
            docker_service: Some("gwt".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
        });

        assert_eq!(summary, "Codex · gpt-5.5 · high · 0.110.0 · docker:gwt");
    }

    #[test]
    fn step_navigation_and_default_selection_follow_runtime_state() {
        let mut docker_context = context(branch("feature/gui"), "feature/gui");
        docker_context.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        docker_context.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state =
            LaunchWizardState::open_with(docker_context, sample_agent_options(), Vec::new());

        state.selected = 1;
        assert_eq!(
            next_step(LaunchWizardStep::BranchAction, &state),
            Some(LaunchWizardStep::BranchTypeSelect)
        );

        state.launch_target = LaunchTargetKind::Shell;
        assert_eq!(
            next_step(LaunchWizardStep::LaunchTarget, &state),
            Some(LaunchWizardStep::RuntimeTarget)
        );

        state.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        assert_eq!(
            next_step(LaunchWizardStep::RuntimeTarget, &state),
            Some(LaunchWizardStep::DockerServiceSelect)
        );
        assert_eq!(
            prev_step(LaunchWizardStep::DockerLifecycle, &state),
            Some(LaunchWizardStep::DockerServiceSelect)
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::DockerServiceSelect, &state),
            1
        );

        state.launch_target = LaunchTargetKind::Agent;
        state.agent_id = "codex".to_string();
        state.model = "gpt-5.5".to_string();
        state.reasoning = "high".to_string();
        state.version = "0.110.0".to_string();
        state.mode = "resume".to_string();
        state.skip_permissions = true;
        state.codex_fast_mode = true;

        assert_eq!(
            next_step(LaunchWizardStep::AgentSelect, &state),
            Some(LaunchWizardStep::ModelSelect)
        );
        assert_eq!(
            next_step(LaunchWizardStep::ModelSelect, &state),
            Some(LaunchWizardStep::ReasoningLevel)
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::ModelSelect, &state),
            current_model_options("codex")
                .iter()
                .position(|model| model == &"gpt-5.5")
                .unwrap()
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::ExecutionMode, &state),
            EXECUTION_MODE_OPTIONS
                .iter()
                .position(|option| option.value == "resume")
                .unwrap()
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::SkipPermissions, &state),
            0
        );
        assert_eq!(
            step_default_selection(LaunchWizardStep::CodexFastMode, &state),
            0
        );
    }

    #[test]
    fn claude_opus_reasoning_options_include_xhigh() {
        let values: Vec<&str> = super::CLAUDE_OPUS_REASONING_OPTIONS
            .iter()
            .map(|option| option.stored_value)
            .collect();
        assert_eq!(
            values,
            ["auto", "low", "medium", "high", "xhigh", "max", "ultracode"]
        );
    }

    #[test]
    fn claude_opus_reasoning_options_include_ultracode_after_max() {
        let values: Vec<&str> = super::CLAUDE_OPUS_REASONING_OPTIONS
            .iter()
            .map(|option| option.stored_value)
            .collect();
        assert_eq!(values.last(), Some(&"ultracode"));
        let max_idx = values.iter().position(|value| *value == "max").unwrap();
        let ultra_idx = values
            .iter()
            .position(|value| *value == "ultracode")
            .unwrap();
        assert!(ultra_idx > max_idx, "ultracode must follow max");
    }

    #[test]
    fn claude_opus_ultracode_is_not_default() {
        let ultra = super::CLAUDE_OPUS_REASONING_OPTIONS
            .iter()
            .find(|option| option.stored_value == "ultracode")
            .expect("opus options must contain ultracode");
        assert!(
            !ultra.is_default,
            "ultracode must be opt-in; auto stays the Opus-tier default"
        );
    }

    #[test]
    fn claude_sonnet_and_codex_reasoning_options_exclude_ultracode() {
        let sonnet: Vec<&str> = super::CLAUDE_SONNET_REASONING_OPTIONS
            .iter()
            .map(|option| option.stored_value)
            .collect();
        // `ultra` is a real Codex effort on 5.6 Sol/Terra; `ultracode` stays a
        // Claude-only session setting and must never appear as a Codex stop.
        let codex: Vec<&str> = codex_reasoning_options_for_model("gpt-5.6-sol")
            .iter()
            .map(|option| option.stored_value)
            .collect();
        assert!(!sonnet.contains(&"ultracode"));
        assert!(!codex.contains(&"ultracode"));
        assert!(codex.contains(&"ultra"));
    }

    #[test]
    fn claude_opus_reasoning_default_is_auto() {
        // Defaulting to Auto skips the CLAUDE_CODE_EFFORT_LEVEL export so
        // Claude Code applies its own per-model default (`high` on
        // Fable 5 / Opus 4.8, `xhigh` on Opus 4.7) regardless of which
        // model the alias resolves to on the user's provider.
        let default = super::CLAUDE_OPUS_REASONING_OPTIONS
            .iter()
            .find(|option| option.is_default)
            .expect("Opus reasoning options must have a default row");
        assert_eq!(default.stored_value, "auto");
    }

    fn claude_state(model: &str, ultracode_supported: bool) -> LaunchWizardState {
        let agent_options = vec![AgentOption {
            id: "claude".to_string(),
            name: "Claude Code".to_string(),
            available: true,
            installed_version: Some("2.1.156 (Claude Code)".to_string()),
            versions: Vec::new(),
            custom_agent: None,
        }];
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        // Installed capability is captured at wizard open, while selected
        // npm versions are evaluated from `state.version`.
        ctx.ultracode_supported = ultracode_supported;
        ctx.claude_workflows_enabled = true;
        let mut state = LaunchWizardState::open_with(ctx, agent_options, Vec::new());
        // Drive current_reasoning_options() down the requested Claude model branch.
        state.agent_id = "claude".to_string();
        state.model = model.to_string();
        state
    }

    fn claude_state_with_version(
        model: &str,
        installed_ultracode_supported: bool,
        version: &str,
    ) -> LaunchWizardState {
        let mut state = claude_state(model, installed_ultracode_supported);
        state.version = version.to_string();
        state
    }

    fn claude_reasoning_values(state: &LaunchWizardState) -> Vec<&'static str> {
        state
            .current_reasoning_options()
            .iter()
            .map(|option| option.stored_value)
            .collect()
    }

    #[test]
    fn opus_reasoning_includes_ultracode_for_installed_when_supported() {
        let values = claude_reasoning_values(&claude_state_with_version("opus", true, "installed"));
        assert!(values.contains(&"ultracode"));
        assert_eq!(values.last(), Some(&"ultracode"));
    }

    #[test]
    fn opus_reasoning_excludes_ultracode_for_installed_when_unsupported() {
        let values =
            claude_reasoning_values(&claude_state_with_version("opus", false, "installed"));
        assert!(!values.contains(&"ultracode"));
        // Common levels remain intact when ultracode is gated out.
        assert!(values.contains(&"xhigh"));
        assert!(values.contains(&"max"));
    }

    #[test]
    fn fable_reasoning_matches_opus_ladder_with_auto_default() {
        let values = claude_reasoning_values(&claude_state("fable", true));
        assert_eq!(
            values,
            ["auto", "low", "medium", "high", "xhigh", "max", "ultracode"]
        );
        let state = claude_state("fable", true);
        let default = state
            .current_reasoning_options()
            .iter()
            .find(|option| option.is_default)
            .map(|option| option.stored_value);
        assert_eq!(default, Some("auto"));
    }

    #[test]
    fn fable_reasoning_excludes_ultracode_for_installed_when_unsupported() {
        let values =
            claude_reasoning_values(&claude_state_with_version("fable", false, "installed"));
        assert!(!values.contains(&"ultracode"));
        assert!(values.contains(&"xhigh"));
        assert!(values.contains(&"max"));
    }

    #[test]
    fn fable_reasoning_includes_ultracode_for_latest_version() {
        let values = claude_reasoning_values(&claude_state_with_version("fable", false, "latest"));
        assert!(values.contains(&"ultracode"));
        assert_eq!(values.last(), Some(&"ultracode"));
    }

    #[test]
    fn fable_reasoning_includes_ultracode_for_supported_pinned_version() {
        let values = claude_reasoning_values(&claude_state_with_version("fable", false, "2.1.154"));
        assert!(values.contains(&"ultracode"));
        assert_eq!(values.last(), Some(&"ultracode"));
    }

    #[test]
    fn fable_reasoning_excludes_ultracode_for_unsupported_pinned_version() {
        let values = claude_reasoning_values(&claude_state_with_version("fable", true, "2.1.153"));
        assert!(!values.contains(&"ultracode"));
        assert!(values.contains(&"xhigh"));
        assert!(values.contains(&"max"));
    }

    #[test]
    fn fable_reasoning_excludes_ultracode_for_latest_when_workflows_disabled() {
        let mut state = claude_state_with_version("fable", true, "latest");
        state.context.claude_workflows_enabled = false;
        let values = claude_reasoning_values(&state);
        assert!(!values.contains(&"ultracode"));
        assert!(values.contains(&"xhigh"));
        assert!(values.contains(&"max"));
    }

    #[test]
    fn fable_is_effort_capable_for_launch() {
        let mut state = claude_state("fable", true);
        state.reasoning = "xhigh".to_string();
        assert_eq!(state.reasoning_level_for_launch(), Some("xhigh"));
    }

    #[test]
    fn claude_sonnet_reasoning_options_exclude_xhigh_and_max() {
        let values: Vec<&str> = super::CLAUDE_SONNET_REASONING_OPTIONS
            .iter()
            .map(|option| option.stored_value)
            .collect();
        assert_eq!(values, ["auto", "low", "medium", "high"]);
        assert!(!values.contains(&"xhigh"));
        assert!(!values.contains(&"max"));
    }

    #[test]
    fn claude_sonnet_reasoning_default_is_auto() {
        // Auto delegates the default effort to Claude Code itself
        // (`high` on Sonnet's current release, per the model-config docs).
        let default = super::CLAUDE_SONNET_REASONING_OPTIONS
            .iter()
            .find(|option| option.is_default)
            .expect("Sonnet reasoning options must have a default row");
        assert_eq!(default.stored_value, "auto");
    }
}
