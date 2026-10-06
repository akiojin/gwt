use super::*;

#[derive(Clone, Copy)]
pub(super) struct ModelDisplayOption {
    pub(super) label: &'static str,
    pub(super) stored_value: &'static str,
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

// SPEC-1921 FR-187: Default stores no model value, so the launch omits the
// model argument and Claude Code picks its own default model.
pub(super) const CLAUDE_DEFAULT_MODEL_VALUE: &str = "";

// SPEC-1921 FR-188: gating reads the stored alias, never the display label.
// Fable shares the Opus effort surface (low..max), and Default resolves to an
// opus-tier model on Claude Code's side, so all three take the same ladder.
pub(super) fn is_claude_opus_tier_model(model: &str) -> bool {
    model == CLAUDE_DEFAULT_MODEL_VALUE || model == "opus" || model == "fable"
}

pub(super) fn is_claude_effort_capable_model(model: &str) -> bool {
    is_claude_opus_tier_model(model) || model == "sonnet"
}

// SPEC-1921 US-28 / FR-186: model generations belong to Claude Code, so the
// picker addresses models only through versionless CLI aliases. The 1m context
// forms, `opusplan`, and `best` stay valid arguments a user can pass
// explicitly; curating the rendered rows does not remove them.
const CLAUDE_MODEL_OPTIONS: [ModelDisplayOption; 5] = [
    ModelDisplayOption {
        label: "Default",
        stored_value: CLAUDE_DEFAULT_MODEL_VALUE,
        description: "Let Claude Code choose its default model",
    },
    ModelDisplayOption {
        label: "Opus",
        stored_value: "opus",
        description: "Deep reasoning for complex problems",
    },
    ModelDisplayOption {
        label: "Fable",
        stored_value: "fable",
        description: "Most capable for the hardest, longest-running tasks",
    },
    ModelDisplayOption {
        label: "Sonnet",
        stored_value: "sonnet",
        description: "Balanced speed and capability",
    },
    ModelDisplayOption {
        label: "Haiku",
        stored_value: "haiku",
        description: "Fastest option for light tasks",
    },
];

#[derive(Clone, Copy)]
pub(super) struct CodexModelCapability {
    pub(super) model: ModelDisplayOption,
    pub(super) default_effort: &'static str,
    pub(super) max_effort: &'static str,
}

// SPEC-1921 US-20 / FR-121..FR-123 + Issue #4795: fixed 2026-09-30 Codex
// picker snapshot, in the CLI's own picker order — the first row is the Codex
// default model. Model rows and reasoning rows both derive from this single
// capability table so stop counts and defaults cannot drift from the model
// list. A later snapshot update edits this table together with the focused
// tests; the wizard never reads a runtime model cache for these rows.
const CODEX_MODEL_CAPABILITIES: [CodexModelCapability; 8] = [
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-6.1-sol",
            stored_value: "gpt-6.1-sol",
            description: "Latest workhorse model for coding and everyday work.",
        },
        default_effort: "low",
        max_effort: "ultra",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-6-astra",
            stored_value: "gpt-6-astra",
            description: "Frontier intelligence for the most demanding work.",
        },
        default_effort: "medium",
        max_effort: "ultra",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-6-sol",
            stored_value: "gpt-6-sol",
            description: "Previous generation workhorse model.",
        },
        default_effort: "medium",
        max_effort: "ultra",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-6-luna",
            stored_value: "gpt-6-luna",
            description: "Fast and affordable model for easier tasks.",
        },
        default_effort: "medium",
        max_effort: "max",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.6-sol",
            stored_value: "gpt-5.6-sol",
            description: "Older generation workhorse model.",
        },
        default_effort: "low",
        max_effort: "ultra",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.6-terra",
            stored_value: "gpt-5.6-terra",
            description: "Older balanced model for straightforward work.",
        },
        default_effort: "medium",
        max_effort: "ultra",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.6-luna",
            stored_value: "gpt-5.6-luna",
            description: "Older fast and efficient model.",
        },
        default_effort: "medium",
        max_effort: "max",
    },
    CodexModelCapability {
        model: ModelDisplayOption {
            label: "gpt-5.5",
            stored_value: "gpt-5.5",
            description: "Legacy coding model.",
        },
        default_effort: "medium",
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
        description: "Balances tokens and intelligence (opus-tier default)",
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

pub(super) const GROK_REASONING_OPTIONS: [ReasoningDisplayOption; 8] = [
    ReasoningDisplayOption {
        label: "Auto",
        stored_value: "auto",
        description: "Use Grok Build's configured default effort",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "None",
        stored_value: "none",
        description: "Disable additional reasoning",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Minimal",
        stored_value: "minimal",
        description: "Use minimal reasoning for the fastest response",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Use light reasoning for simple work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balance speed and reasoning depth",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Use deeper reasoning for complex work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "xHigh",
        stored_value: "xhigh",
        description: "Use extra-high reasoning depth",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Max",
        stored_value: "max",
        description: "Use maximum reasoning depth",
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
        .find(|capability| capability.model.stored_value == model);
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
                } else if self.state.agent_uses_reasoning_step() {
                    Some(LaunchWizardStep::ReasoningLevel)
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
            LaunchWizardStep::ReasoningLevel => {
                if self.state.agent_has_models() {
                    Some(LaunchWizardStep::ModelSelect)
                } else {
                    Some(LaunchWizardStep::AgentSelect)
                }
            }
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
        } else {
            Some(LaunchWizardStep::SkipPermissions)
        }
    }

    fn next_after_docker_lifecycle(&self) -> Option<LaunchWizardStep> {
        self.next_after_windows_shell()
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

    fn previous_before_execution_mode(&self) -> Option<LaunchWizardStep> {
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
        LaunchWizardStep::ReasoningLevel => {
            let options = state.current_reasoning_options();
            options
                .iter()
                .position(|option| option.stored_value == state.reasoning)
                .or_else(|| state.unlisted_grok_reasoning().map(|_| options.len()))
                .unwrap_or_else(|| {
                    options
                        .iter()
                        .position(|option| option.is_default)
                        .unwrap_or(0)
                })
        }
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
    model_display_options(agent_id)
        .iter()
        .map(|option| option.stored_value)
        .collect()
}

pub(super) fn model_display_options(agent_id: &str) -> &'static [ModelDisplayOption] {
    match agent_id {
        "claude" => &CLAUDE_MODEL_OPTIONS,
        "codex" => &CODEX_MODEL_OPTIONS,
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
        crate::WindowProcessStatus::Interrupted => "interrupted",
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

/// SPEC-3864 FR-005..FR-007: what the wizard offers when an agent cannot
/// launch as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentSetupKind {
    /// Install a missing executable ahead of launch.
    Install,
    /// Refresh an installed CLI before launching.
    Update,
    /// The executable is launchable but first-time configuration is missing.
    Configure,
}

impl AgentSetupKind {
    pub fn wire_value(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Update => "update",
            Self::Configure => "configure",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSetupAffordance {
    pub kind: AgentSetupKind,
    pub title: String,
    pub detail: String,
    /// Button label when gwt can run the setup action; `None` when
    /// the user has to act outside gwt.
    pub action_label: Option<String>,
}

pub(super) fn agent_install_update_command(
    descriptor: &gwt_agent::BuiltinAgentDescriptor,
    update: bool,
) -> Option<String> {
    match descriptor.id {
        gwt_agent::AgentId::ClaudeCode if update => Some("claude update".to_string()),
        gwt_agent::AgentId::ClaudeCode if cfg!(windows) => Some(
            r#"powershell -NoProfile -Command "irm https://claude.ai/install.ps1 | iex""#
                .to_string(),
        ),
        gwt_agent::AgentId::ClaudeCode => {
            Some("curl -fsSL https://claude.ai/install.sh | bash".to_string())
        }
        gwt_agent::AgentId::Codex => Some("npm install -g @openai/codex".to_string()),
        _ => descriptor.distribution.install_shell_command(),
    }
}

/// Derive setup from distribution and detection state (SPEC-3864 FR-006).
/// Claude and Codex expose pre-install/update commands for their installed-only
/// Host launch (SPEC-1921).
pub fn agent_setup_affordance(
    descriptor: &gwt_agent::BuiltinAgentDescriptor,
    available: bool,
    needs_configuration: bool,
) -> Option<AgentSetupAffordance> {
    let name = descriptor.display_name;
    if matches!(
        descriptor.id,
        gwt_agent::AgentId::ClaudeCode | gwt_agent::AgentId::Codex
    ) {
        let verb = if available { "Update" } else { "Install" };
        let command = agent_install_update_command(descriptor, available)?;
        return Some(AgentSetupAffordance {
            kind: if available {
                AgentSetupKind::Update
            } else {
                AgentSetupKind::Install
            },
            title: format!("{verb} {name} before launch"),
            detail: if available {
                format!("Run `{command}` in the background on the Host. View progress and the updated version here; your Agent Settings stay open.")
            } else {
                format!("Run `{command}` in a host shell pane. Restart gwt afterward to refresh the detected version. Launch uses the detected CLI directly and reports an error if it cannot launch.")
            },
            action_label: Some(format!("{verb} {name}")),
        });
    }
    if !available && !descriptor.distribution.supports_runtime_latest() {
        let title = format!("{name} is not installed");
        return Some(match descriptor.distribution.install_shell_command() {
            Some(command) => AgentSetupAffordance {
                kind: AgentSetupKind::Install,
                title,
                detail: format!(
                    "Install it with: {command} — gwt can run this in a shell pane. \
                     Restart gwt after installing so `{}` is detected on PATH.",
                    descriptor.command
                ),
                action_label: Some(format!("Install {name}")),
            },
            None => AgentSetupAffordance {
                kind: AgentSetupKind::Install,
                title,
                detail: format!(
                    "No distribution route is known. Install `{}` manually, put it on PATH, \
                     and restart gwt.",
                    descriptor.command
                ),
                action_label: None,
            },
        });
    }
    if needs_configuration && !descriptor.setup_args.is_empty() {
        let setup_command = std::iter::once(descriptor.command)
            .chain(descriptor.setup_args.iter().copied())
            .collect::<Vec<_>>()
            .join(" ");
        return Some(AgentSetupAffordance {
            kind: AgentSetupKind::Configure,
            title: format!("{name} is not set up yet"),
            detail: format!(
                "Run `{setup_command}` to finish first-time setup. You can still launch — \
                 {name} will prompt for setup."
            ),
            action_label: Some(format!("Run {name} setup")),
        });
    }
    None
}

/// Shown when no supported agent is installed, so the empty agent list says
/// what to do next (SPEC-1921 AS-1921-B).
pub(super) const NO_DETECTED_AGENT_TITLE: &str = "No supported agent CLI was detected";

pub(super) fn no_detected_agent_setup_view() -> super::LaunchWizardAgentSetupView {
    super::LaunchWizardAgentSetupView {
        agent_id: String::new(),
        kind: AgentSetupKind::Install.wire_value().to_string(),
        title: NO_DETECTED_AGENT_TITLE.to_string(),
        detail: "Install one of the agents listed under Supported agents in the README, \
                 make sure it is on PATH, then reopen this wizard. Shell launches stay available."
            .to_string(),
        action_label: None,
        pending: false,
        status: None,
    }
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
        None if agent.available => "Detected".to_string(),
        // SPEC-3864 FR-004: `available` is the real detection result, so an
        // undetected built-in says so instead of the neutral "Built-in".
        None => "Not installed".to_string(),
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

/// The agents the Launch Wizard offers: detected built-ins followed by the
/// configured custom agents (SPEC-1921 FR-1921-L6). A built-in that is not
/// installed is left out, because the wizard launches the resolved executable
/// and has nothing to start for it.
pub fn build_agent_options(
    detected_agents: Vec<gwt_agent::DetectedAgent>,
    custom_agents: Vec<gwt_agent::CustomCodingAgent>,
) -> Vec<AgentOption> {
    let mut options = build_builtin_agent_options(detected_agents);
    options.retain(|option| option.available);
    options.extend(custom_agents.into_iter().map(|agent| AgentOption {
        id: agent.id.clone(),
        name: agent.display_name.clone(),
        available: true,
        installed_version: None,
        custom_agent: Some(agent),
    }));
    options
}

/// Production wizard entry point: runs install detection for every built-in
/// (SPEC-3864 FR-002) so `available` / `installed_version` reflect the host
/// instead of an empty detection list.
pub fn load_agent_options() -> Vec<AgentOption> {
    let environment = crate::profile_dispatch::config_path()
        .map_err(|error| error.to_string())
        .and_then(|path| {
            gwt_agent::LaunchEnvironment::from_active_profile(
                &path,
                gwt_agent::LaunchRuntimeTarget::Host,
            )
        })
        .map(gwt_agent::LaunchEnvironment::into_parts);
    if let Err(error) = &environment {
        tracing::warn!(%error, "cannot detect installed agents in the active profile");
    }
    build_agent_options(
        detect_wizard_agents(environment.as_ref().ok()),
        load_global_custom_agents(),
    )
}

fn detect_wizard_agents(
    environment: Option<&(std::collections::HashMap<String, String>, Vec<String>)>,
) -> Vec<gwt_agent::DetectedAgent> {
    std::thread::scope(|scope| {
        let probes: Vec<_> = gwt_agent::builtin_agent_descriptors()
            .iter()
            .map(|descriptor| {
                scope.spawn(move || {
                    if matches!(
                        descriptor.id,
                        gwt_agent::AgentId::ClaudeCode | gwt_agent::AgentId::Codex
                    ) {
                        let (env, remove_env) = environment?;
                        gwt_agent::AgentDetector::detect_by_command_with_environment(
                            descriptor.command,
                            env,
                            remove_env,
                            None,
                        )
                    } else {
                        gwt_agent::AgentDetector::detect_by_command(descriptor.command)
                    }
                })
            })
            .collect();
        probes
            .into_iter()
            .filter_map(|probe| probe.join().ok().flatten())
            .collect()
    })
}

/// Every built-in agent with its detection result. Undetected built-ins are
/// included with `available: false`; [`build_agent_options`] drops them for
/// the wizard.
pub fn build_builtin_agent_options(
    detected_agents: Vec<gwt_agent::DetectedAgent>,
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
                // SPEC-3864 FR-004: derived from detection, never hardcoded.
                available: detected.is_some(),
                installed_version: detected.and_then(|detected| detected.version.clone()),
                custom_agent: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use tempfile::tempdir;

    use super::super::test_support::*;
    use super::*;

    fn grok_manual_state() -> LaunchWizardState {
        let mut agents = sample_agent_options();
        agents.push(AgentOption {
            id: "grok".to_string(),
            name: "Grok Build".to_string(),
            available: true,
            installed_version: Some("1.0.3".to_string()),
            custom_agent: None,
        });
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/grok"), "feature/grok"),
            agents,
            Vec::new(),
        );
        state.set_agent_id("grok");
        state
    }

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
            agent_option_color("grok"),
            Some(gwt_agent::AgentColor::Gray)
        );
        assert_eq!(
            agent_option_color("gemini"),
            Some(gwt_agent::AgentColor::Gray)
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

    /// SPEC-3864 FR-002 / FR-004 (AC-2 / AC-4): the production wizard entry
    /// point must run real install detection and derive `available` from it.
    /// A fake `agy` on PATH is the only detectable built-in, so Antigravity
    /// must come back available with its probed version while every other
    /// built-in is excluded from the launch choices.
    #[cfg(unix)]
    #[test]
    fn load_agent_options_runs_detection_and_derives_availability() {
        use std::os::unix::fs::PermissionsExt;

        let _env = gwt_core::test_support::env_lock()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempdir().expect("tempdir");
        let executable = dir.path().join("agy");
        std::fs::write(&executable, "#!/bin/sh\nprintf '1.2.3\\n'\n").expect("write agy stub");
        let mut permissions = std::fs::metadata(&executable)
            .expect("stub metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&executable, permissions).expect("chmod stub");
        // PATH is replaced wholesale so no real agent leaks in, but tests that
        // spawn `git` or `sh` without the env lock still run concurrently;
        // keep both reachable through the scoped PATH (Issue #4497).
        let git = which::which("git").expect("git on the test runner");
        std::os::unix::fs::symlink(&git, dir.path().join("git")).expect("link git");
        let sh = which::which("sh").expect("sh on the test runner");
        std::os::unix::fs::symlink(&sh, dir.path().join("sh")).expect("link sh");
        let _path = gwt_core::test_support::ScopedEnvVar::set("PATH", dir.path());
        assert!(
            which::which("sh").is_ok(),
            "scoped PATH must retain sh for parallel runner probes"
        );
        let _no_custom = gwt_core::test_support::ScopedEnvVar::set(
            gwt_agent::DISABLE_GLOBAL_CUSTOM_AGENTS_ENV,
            "1",
        );

        let _home = gwt_core::test_support::ScopedEnvVar::set("HOME", dir.path());
        let _userprofile = gwt_core::test_support::ScopedEnvVar::set("USERPROFILE", dir.path());
        let mut settings = gwt_config::Settings::default();
        settings.profiles.normalize_active_profile();
        settings.profiles.profiles[0]
            .env_vars
            .insert("PATH".into(), dir.path().to_string_lossy().into_owned());
        settings
            .save(&gwt_config::Settings::global_config_path_for_home(
                dir.path(),
            ))
            .unwrap();
        let options = load_agent_options();

        let agy = options
            .iter()
            .find(|option| option.id == "agy")
            .expect("Antigravity option");
        assert!(agy.available, "detected agent must be available");
        assert_eq!(agy.installed_version.as_deref(), Some("1.2.3"));
        assert_eq!(
            options
                .iter()
                .map(|option| option.id.as_str())
                .collect::<Vec<_>>(),
            ["agy"],
            "undetected built-ins must be absent from launch choices"
        );
    }

    #[test]
    fn build_builtin_agent_options_marks_undetected_builtins_unavailable() {
        // SPEC-3864 FR-004: `available` is a detection result, not a label.
        let options = build_builtin_agent_options(vec![gwt_agent::DetectedAgent {
            agent_id: gwt_agent::AgentId::OpenClaw,
            version: Some("2026.1.0".to_string()),
            path: PathBuf::from("/opt/homebrew/bin/openclaw"),
        }]);
        let openclaw = options
            .iter()
            .find(|option| option.id == "openclaw")
            .expect("OpenClaw option");
        assert!(openclaw.available);
        assert_eq!(openclaw.installed_version.as_deref(), Some("2026.1.0"));
        assert_eq!(agent_description(openclaw), "Detected · 2026.1.0");

        let agy = options
            .iter()
            .find(|option| option.id == "agy")
            .expect("Antigravity option");
        assert!(!agy.available);
        assert_eq!(agent_description(agy), "Not installed");
    }

    /// SPEC-3864 FR-005 (AC-5): every pre-install built-in (no runtime
    /// `latest` route) surfaces an install affordance when it is missing, and
    /// npm-routed or detected agents surface none.
    #[test]
    fn agent_setup_affordance_covers_preinstall_builtins_when_missing() {
        for command in ["agy", "hermes", "gh"] {
            let descriptor = gwt_agent::builtin_agent_descriptor_for_command(command)
                .expect("built-in descriptor");
            let affordance = agent_setup_affordance(descriptor, false, false)
                .unwrap_or_else(|| panic!("{command} must offer an install affordance"));
            assert_eq!(affordance.kind, AgentSetupKind::Install, "{command}");
            assert!(
                affordance.title.contains(descriptor.display_name),
                "{command}: {}",
                affordance.title
            );
            let install_command = descriptor
                .distribution
                .install_shell_command()
                .expect("install command");
            assert!(
                affordance.detail.contains(&install_command),
                "{command}: {}",
                affordance.detail
            );
            assert!(affordance.action_label.is_some(), "{command}");
        }
        for command in ["openclaw", "opencode"] {
            let descriptor = gwt_agent::builtin_agent_descriptor_for_command(command)
                .expect("built-in descriptor");
            assert_eq!(
                agent_setup_affordance(descriptor, false, false),
                None,
                "{command} can run latest at launch and needs no install affordance"
            );
            assert_eq!(
                agent_setup_affordance(descriptor, true, false),
                None,
                "{command}"
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn wizard_detection_uses_the_active_profile_path() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        for (directory, version) in [("old", "2.1.153"), ("new", "2.1.156")] {
            let bin = temp.path().join(directory);
            std::fs::create_dir(&bin).unwrap();
            let executable = bin.join("claude");
            std::fs::write(&executable, format!("#!/bin/sh\nprintf '{version}\\n'\n")).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
            let environment = (
                std::collections::HashMap::from([(
                    "PATH".into(),
                    bin.to_string_lossy().into_owned(),
                )]),
                Vec::new(),
            );
            let detected = detect_wizard_agents(Some(&environment));
            let options = build_agent_options(detected, Vec::new());
            let claude = options.iter().find(|agent| agent.id == "claude").unwrap();
            assert!(claude.available);
            assert_eq!(claude.installed_version.as_deref(), Some(version));
        }
    }

    #[test]
    fn installed_agents_offer_install_and_update_without_package_fallback() {
        for command in ["claude", "codex"] {
            let descriptor = gwt_agent::builtin_agent_descriptor_for_command(command).unwrap();
            let install = agent_setup_affordance(descriptor, false, false).expect("install");
            assert_eq!(install.kind.wire_value(), "install");
            let update = agent_setup_affordance(descriptor, true, false).expect("update");
            assert_eq!(update.kind.wire_value(), "update");
            for affordance in [install, update] {
                assert!(
                    !affordance.detail.contains("package runner"),
                    "installed-only guidance must not promise package fallback: {}",
                    affordance.detail
                );
            }
        }
    }

    /// SPEC-3864 FR-006 / FR-007 (AC-6): the affordance is derived from the
    /// descriptor alone. A synthetic agent that exists nowhere else in gwt
    /// gets both the install and the configure affordance without any
    /// agent-specific branch.
    #[test]
    fn agent_setup_affordance_is_descriptor_driven_for_synthetic_agent() {
        let descriptor = gwt_agent::BuiltinAgentDescriptor {
            id: gwt_agent::AgentId::Custom("zeta-cli".to_string()),
            command: "zeta",
            display_name: "Zeta CLI",
            distribution: gwt_agent::DistributionRoute::Homebrew {
                formula: "zeta/tap/zeta",
            },
            setup_args: &["login"],
            color: gwt_agent::AgentColor::Gray,
            aliases: &["zeta"],
            cache_key: "zeta",
            version_flag: "--version",
            version_prefix_args: &[],
        };

        let install = agent_setup_affordance(&descriptor, false, false).expect("install");
        assert_eq!(install.kind, AgentSetupKind::Install);
        assert!(install.title.contains("Zeta CLI"), "{}", install.title);
        assert!(
            install.detail.contains("brew install zeta/tap/zeta"),
            "{}",
            install.detail
        );
        assert_eq!(install.action_label.as_deref(), Some("Install Zeta CLI"));

        let configure = agent_setup_affordance(&descriptor, true, true).expect("configure");
        assert_eq!(configure.kind, AgentSetupKind::Configure);
        assert!(
            configure.detail.contains("zeta login"),
            "{}",
            configure.detail
        );
        assert_eq!(
            configure.action_label.as_deref(),
            Some("Run Zeta CLI setup")
        );

        assert_eq!(agent_setup_affordance(&descriptor, true, false), None);
    }

    /// SPEC-3864 FR-005: an agent with no known route still tells the user it
    /// is missing, but offers no runnable action.
    #[test]
    fn agent_setup_affordance_without_route_has_no_action() {
        let descriptor = gwt_agent::BuiltinAgentDescriptor {
            id: gwt_agent::AgentId::Custom("omega".to_string()),
            command: "omega",
            display_name: "Omega",
            distribution: gwt_agent::DistributionRoute::None,
            setup_args: &[],
            color: gwt_agent::AgentColor::Gray,
            aliases: &[],
            cache_key: "omega",
            version_flag: "--version",
            version_prefix_args: &[],
        };
        let affordance = agent_setup_affordance(&descriptor, false, false).expect("install");
        assert_eq!(affordance.kind, AgentSetupKind::Install);
        assert_eq!(affordance.action_label, None);
        assert!(affordance.detail.contains("omega"), "{}", affordance.detail);
    }

    #[test]
    fn build_builtin_agent_options_includes_hook_parity_agents() {
        let options = build_builtin_agent_options(Vec::new());
        let ids: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();

        assert_eq!(
            ids,
            vec!["claude", "codex", "grok", "agy", "opencode", "openclaw", "hermes", "gh"]
        );
        assert!(options.iter().any(|option| option.name == "Grok Build"));
        assert!(options
            .iter()
            .any(|option| option.name == "Antigravity CLI"));
        assert!(options.iter().all(|option| !option.id.contains("gemini")));
        assert!(options.iter().any(|option| option.name == "OpenCode"));
        assert!(options.iter().any(|option| option.name == "OpenClaw"));
        assert!(options.iter().any(|option| option.name == "Hermes Agent"));
    }

    #[test]
    fn build_builtin_agent_options_projects_detected_grok_version() {
        let options = build_builtin_agent_options(vec![gwt_agent::DetectedAgent {
            agent_id: gwt_agent::AgentId::GrokBuild,
            version: Some("1.0.3".to_string()),
            path: PathBuf::from("/usr/local/bin/grok"),
        }]);
        let grok = options
            .iter()
            .find(|option| option.id == "grok")
            .expect("Grok Build option");

        assert_eq!(grok.name, "Grok Build");
        assert!(grok.available);
        assert_eq!(grok.installed_version.as_deref(), Some("1.0.3"));
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
        state.agent_id = "opencode".to_string();

        let view = state.view();
        assert!(
            view.execution_mode_options
                .iter()
                .all(|option| option.value != "resume"),
            "OpenCode must not advertise the picker option: {:?}",
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
            vec!["", "opus", "fable", "sonnet", "haiku"]
        );
        assert_eq!(
            current_model_options("codex"),
            vec![
                "gpt-6.1-sol",
                "gpt-6-astra",
                "gpt-6-sol",
                "gpt-6-luna",
                "gpt-5.6-sol",
                "gpt-5.6-terra",
                "gpt-5.6-luna",
                "gpt-5.5",
            ]
        );
        assert!(current_model_options("gemini").is_empty());
        assert!(model_display_options("gemini").is_empty());
        assert!(current_model_options("agy").is_empty());
        assert!(model_display_options("agy").is_empty());
        assert!(current_model_options("custom").is_empty());
        assert!(model_display_options("custom").is_empty());
        assert!(!model_display_options("codex").is_empty());
    }

    // SPEC-1921 US-20 / FR-121 + Issue #4795 AC-1/3/4: visible rows from
    // the 2026-09-30 Codex v0.159.2 picker, in the CLI's order.
    #[test]
    fn codex_model_catalog_matches_2026_09_30_snapshot() {
        let rows: Vec<(&str, &str)> = model_display_options("codex")
            .iter()
            .map(|option| (option.label, option.description))
            .collect();
        assert_eq!(
            rows,
            vec![
                (
                    "gpt-6.1-sol",
                    "Latest workhorse model for coding and everyday work."
                ),
                (
                    "gpt-6-astra",
                    "Frontier intelligence for the most demanding work."
                ),
                ("gpt-6-sol", "Previous generation workhorse model."),
                ("gpt-6-luna", "Fast and affordable model for easier tasks."),
                ("gpt-5.6-sol", "Older generation workhorse model."),
                (
                    "gpt-5.6-terra",
                    "Older balanced model for straightforward work."
                ),
                ("gpt-5.6-luna", "Older fast and efficient model."),
                ("gpt-5.5", "Legacy coding model."),
            ]
        );
        // Exact equality also excludes retired models and hidden cache rows
        // (gpt-reserve / codex-auto-review).
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

    // SPEC-1921 US-20 / FR-122 + FR-123 + Issue #3962 AC-3: reasoning rows and
    // the initial stop derive from the selected model's capability row. The
    // expectations below mirror the CLI's own effort picker
    // (`supported_reasoning_levels` / `default_reasoning_level`), so Astra /
    // Sol / Terra expose six stops through Ultra, Luna five through Max, and
    // gpt-5.5 four through Extra high. gpt-6.1-sol and gpt-5.6-sol default
    // to Low; every other visible model defaults to Medium.
    #[test]
    fn codex_reasoning_capability_rows_follow_model() {
        const SIX: [&str; 6] = ["low", "medium", "high", "xhigh", "max", "ultra"];
        const FIVE: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
        const FOUR: [&str; 4] = ["low", "medium", "high", "xhigh"];

        assert_eq!(codex_capability_row("gpt-6.1-sol"), (SIX.to_vec(), "low"));
        assert_eq!(
            codex_capability_row("gpt-6-astra"),
            (SIX.to_vec(), "medium")
        );
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
        assert_eq!(codex_capability_row("gpt-6-sol"), (SIX.to_vec(), "medium"));
        assert_eq!(
            codex_capability_row("gpt-6-luna"),
            (FIVE.to_vec(), "medium")
        );

        // Every catalog row must be covered by the expectations above, so a
        // future snapshot cannot add a model whose effort ladder goes untested.
        let covered = [
            "gpt-6.1-sol",
            "gpt-6-astra",
            "gpt-6-sol",
            "gpt-6-luna",
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
        ];
        assert_eq!(current_model_options("codex"), covered.to_vec());
    }

    #[test]
    fn grok_build_reasoning_options_cover_common_effort_values_with_auto_default() {
        // SPEC-1921 T483: these stored values are the launch contract. `auto`
        // delegates to the Grok CLI/config and every other row maps verbatim
        // to `--effort <LEVEL>`.
        let options = grok_manual_state().current_reasoning_options();
        let values: Vec<&str> = options.iter().map(|option| option.stored_value).collect();

        assert_eq!(
            values,
            ["auto", "none", "minimal", "low", "medium", "high", "xhigh", "max"]
        );
        assert_eq!(
            options
                .iter()
                .filter(|option| option.is_default)
                .map(|option| option.stored_value)
                .collect::<Vec<_>>(),
            ["auto"]
        );
        assert!(
            current_model_options("grok").is_empty(),
            "Grok model entry is free text, not a fixed catalog"
        );

        let state = grok_manual_state();
        let flow = LaunchWizardFlow::new(&state);
        assert_eq!(
            flow.next_step(LaunchWizardStep::AgentSelect),
            Some(LaunchWizardStep::ReasoningLevel),
            "free-text model is edited in the form, but legacy flow must still visit effort"
        );
        assert_eq!(
            flow.prev_step(LaunchWizardStep::ReasoningLevel),
            Some(LaunchWizardStep::AgentSelect),
            "Grok has no fixed ModelSelect step to return to"
        );
    }

    // Unknown or legacy persisted Codex models keep the conservative pre-5.6
    // surface so a stale saved model can never unlock unsupported stops.
    // Issue #3962: `gpt-5.4` joined that legacy set when it left the picker.
    #[test]
    fn codex_reasoning_capability_falls_back_conservatively_for_unknown_model() {
        let (values, default) = codex_capability_row("gpt-5.2-codex");
        assert_eq!(values, vec!["low", "medium", "high", "xhigh"]);
        assert_eq!(default, "medium");

        let (values, default) = codex_capability_row("gpt-5.4");
        assert_eq!(values, vec!["low", "medium", "high", "xhigh"]);
        assert_eq!(default, "medium");
    }

    #[test]
    fn quick_start_summary_includes_runtime_metadata() {
        let summary = quick_start_summary(&QuickStartEntry {
            session_id: "gwt-session-1".to_string(),
            linked_issue_number: None,
            agent_id: "codex".to_string(),
            tool_label: "Codex".to_string(),
            model: Some("gpt-5.5".to_string()),
            reasoning: Some("high".to_string()),
            resume_session_id: Some("resume-1".to_string()),
            live_window_id: None,
            skip_permissions: true,
            codex_fast_mode: true,
            runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
            docker_service: Some("gwt".to_string()),
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
        });

        assert_eq!(summary, "Codex · gpt-5.5 · high · docker:gwt");
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
            installed_version: Some(
                if ultracode_supported {
                    "2.1.156 (Claude Code)"
                } else {
                    "2.1.153 (Claude Code)"
                }
                .to_string(),
            ),
            custom_agent: None,
        }];
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        // Detection supplies installed capabilities. The legacy snapshot is
        // intentionally independent of the detected version.
        ctx.ultracode_supported = ultracode_supported;
        ctx.claude_workflows_enabled = true;
        let mut state = LaunchWizardState::open_with(ctx, agent_options, Vec::new());
        // Drive current_reasoning_options() down the requested Claude model branch.
        state.agent_id = "claude".to_string();
        state.model = model.to_string();
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
        let values = claude_reasoning_values(&claude_state("opus", true));
        assert!(values.contains(&"ultracode"));
        assert_eq!(values.last(), Some(&"ultracode"));
    }

    #[test]
    fn opus_reasoning_excludes_ultracode_for_installed_when_unsupported() {
        let values = claude_reasoning_values(&claude_state("opus", false));
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
        let values = claude_reasoning_values(&claude_state("fable", false));
        assert!(!values.contains(&"ultracode"));
        assert!(values.contains(&"xhigh"));
        assert!(values.contains(&"max"));
    }

    #[test]
    fn fable_reasoning_excludes_ultracode_when_workflows_disabled() {
        let mut state = claude_state("fable", true);
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
    // SPEC-1921 Phase 77 (US-28 / AS-VCM-01; FR-186..FR-187; SC-065):
    // the picker shows five versionless rows in a fixed order and each row
    // stores the CLI alias rather than its display label. Default stores no
    // model value at all, so the launch omits the model argument.
    #[test]
    fn claude_model_rows_are_versionless_and_ordered() {
        let labels: Vec<&str> = model_display_options("claude")
            .iter()
            .map(|option| option.label)
            .collect();
        assert_eq!(labels, ["Default", "Opus", "Fable", "Sonnet", "Haiku"]);
        assert_eq!(
            current_model_options("claude"),
            vec!["", "opus", "fable", "sonnet", "haiku"]
        );
        assert!(!is_explicit_model_selection(""));
    }

    // SPEC-1921 Phase 77 (AS-VCM-02; FR-186; SC-065): model generations belong
    // to Claude Code, so no rendered Claude string may pin one.
    #[test]
    fn claude_display_strings_carry_no_version_numbers() {
        let mut strings: Vec<&str> = Vec::new();
        for option in model_display_options("claude") {
            strings.push(option.label);
            strings.push(option.description);
        }
        for option in super::CLAUDE_OPUS_REASONING_OPTIONS
            .iter()
            .chain(super::CLAUDE_SONNET_REASONING_OPTIONS.iter())
        {
            strings.push(option.label);
            strings.push(option.description);
        }
        for text in strings {
            assert!(
                !text.chars().any(|character| character.is_ascii_digit()),
                "Claude display string must not pin a model version: {text}"
            );
        }
    }

    // SPEC-1921 Phase 77 (AS-VCM-05; FR-188): effort gating reads the stored
    // identifier, so a display label can never unlock the opus-tier ladder.
    #[test]
    fn claude_effort_gating_keys_off_stored_identifier() {
        assert!(is_claude_opus_tier_model(""));
        assert!(is_claude_opus_tier_model("opus"));
        assert!(is_claude_opus_tier_model("fable"));
        assert!(!is_claude_opus_tier_model("sonnet"));
        assert!(!is_claude_opus_tier_model("haiku"));
        assert!(!is_claude_opus_tier_model("Opus"));
        assert!(!is_claude_opus_tier_model("Default (Opus 4.8)"));

        assert!(is_claude_effort_capable_model(""));
        assert!(is_claude_effort_capable_model("sonnet"));
        assert!(!is_claude_effort_capable_model("haiku"));
        assert!(!is_claude_effort_capable_model("Sonnet"));
    }

    // SPEC-1921 Phase 77 (AS-VCM-05): Default follows the opus-tier ladder
    // because it resolves to Claude Code's own default model, and Haiku still
    // has no effort step at all.
    #[test]
    fn claude_default_row_uses_opus_ladder_and_haiku_skips_effort() {
        let default_state = claude_state("", true);
        assert!(default_state.agent_uses_reasoning_step());
        assert_eq!(
            claude_reasoning_values(&default_state),
            ["auto", "low", "medium", "high", "xhigh", "max", "ultracode"]
        );
        assert!(!claude_state("haiku", true).agent_uses_reasoning_step());
    }

    /// SPEC-1921 AS-1921-B (AC-1921-L4): the wizard lists installed agents
    /// only. An undetected built-in is absent rather than shown as
    /// "Not installed"; configured custom agents stay.
    #[test]
    fn build_agent_options_lists_only_detected_builtins_and_custom_agents() {
        let options = build_agent_options(
            vec![gwt_agent::DetectedAgent {
                agent_id: gwt_agent::AgentId::Codex,
                version: Some("0.159.2".to_string()),
                path: PathBuf::from("/opt/homebrew/bin/codex"),
            }],
            vec![sample_custom_agent(
                "proxy-agent",
                "Claude Proxy",
                gwt_agent::custom::CustomAgentType::Command,
                "proxy-agent",
            )],
        );

        let ids: Vec<&str> = options.iter().map(|option| option.id.as_str()).collect();
        assert_eq!(ids, ["codex", "proxy-agent"]);
        assert!(options.iter().all(|option| option.available));
    }

    /// SPEC-1921 AS-1921-B: nothing detected and nothing configured is an
    /// empty list, not a list of unlaunchable built-ins.
    #[test]
    fn build_agent_options_is_empty_when_nothing_is_detected() {
        let options = build_agent_options(Vec::new(), Vec::new());
        assert!(options.is_empty(), "{options:?}");
    }
}
