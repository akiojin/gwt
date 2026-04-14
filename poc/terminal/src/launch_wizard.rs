use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::BranchListEntry;

const DEFAULT_NEW_BRANCH_BASE_BRANCH: &str = "develop";
const BRANCH_TYPE_PREFIXES: [&str; 4] = ["feature/", "bugfix/", "hotfix/", "release/"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaunchWizardStep {
    QuickStart,
    FocusExistingSession,
    BranchAction,
    BranchTypeSelect,
    BranchNameInput,
    AgentSelect,
    ModelSelect,
    ReasoningLevel,
    RuntimeTarget,
    DockerServiceSelect,
    DockerLifecycle,
    VersionSelect,
    ExecutionMode,
    SkipPermissions,
    CodexFastMode,
}

impl LaunchWizardStep {
    fn title(self) -> &'static str {
        match self {
            Self::QuickStart => "Quick Start",
            Self::FocusExistingSession => "Focus Existing Session",
            Self::BranchAction => "Branch Action",
            Self::BranchTypeSelect => "Branch Type",
            Self::BranchNameInput => "Branch Name",
            Self::AgentSelect => "Select Agent",
            Self::ModelSelect => "Select Model",
            Self::ReasoningLevel => "Reasoning Level",
            Self::RuntimeTarget => "Runtime Target",
            Self::DockerServiceSelect => "Docker Service",
            Self::DockerLifecycle => "Docker Lifecycle",
            Self::VersionSelect => "Select Version",
            Self::ExecutionMode => "Execution Mode",
            Self::SkipPermissions => "Skip Permissions",
            Self::CodexFastMode => "Codex Fast Mode",
        }
    }
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardOptionView {
    pub label: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct LaunchWizardView {
    pub step: LaunchWizardStep,
    pub title: String,
    pub branch_name: String,
    pub selected: usize,
    pub step_index: usize,
    pub step_count: usize,
    pub can_go_back: bool,
    pub options: Vec<LaunchWizardOptionView>,
    pub input_value: Option<String>,
    pub input_label: Option<String>,
    pub input_placeholder: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOption {
    pub id: String,
    pub name: String,
    pub available: bool,
    pub installed_version: Option<String>,
    pub versions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickStartEntry {
    pub agent_id: String,
    pub tool_label: String,
    pub model: Option<String>,
    pub reasoning: Option<String>,
    pub version: Option<String>,
    pub resume_session_id: Option<String>,
    pub skip_permissions: bool,
    pub codex_fast_mode: bool,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub docker_service: Option<String>,
    pub docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveSessionEntry {
    pub session_id: String,
    pub window_id: String,
    pub kind: String,
    pub name: String,
    pub detail: Option<String>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DockerWizardContext {
    pub services: Vec<String>,
    pub suggested_service: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LaunchWizardContext {
    pub selected_branch: BranchListEntry,
    pub normalized_branch_name: String,
    pub worktree_path: Option<PathBuf>,
    pub quick_start_root: PathBuf,
    pub live_sessions: Vec<LiveSessionEntry>,
    pub docker_context: Option<DockerWizardContext>,
    pub docker_service_status: gwt_docker::ComposeServiceStatus,
}

#[derive(Debug, Clone)]
pub enum LaunchWizardCompletion {
    Launch(Box<gwt_agent::LaunchConfig>),
    FocusWindow { window_id: String },
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LaunchWizardAction {
    Select { index: usize },
    Back,
    Cancel,
    SubmitText { value: String },
}

#[derive(Debug, Clone)]
pub struct LaunchWizardState {
    pub context: LaunchWizardContext,
    pub step: LaunchWizardStep,
    pub selected: usize,
    pub detected_agents: Vec<AgentOption>,
    pub quick_start_entries: Vec<QuickStartEntry>,
    pub is_new_branch: bool,
    pub base_branch_name: Option<String>,
    pub agent_id: String,
    pub model: String,
    pub reasoning: String,
    pub version: String,
    pub mode: String,
    pub resume_session_id: Option<String>,
    pub runtime_target: gwt_agent::LaunchRuntimeTarget,
    pub docker_service: Option<String>,
    pub docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent,
    pub skip_permissions: bool,
    pub codex_fast_mode: bool,
    pub branch_name: String,
    pub completion: Option<LaunchWizardCompletion>,
    pub error: Option<String>,
}

impl LaunchWizardState {
    pub fn open_with(
        context: LaunchWizardContext,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
    ) -> Self {
        let runtime_target = if context.docker_context.is_some() {
            gwt_agent::LaunchRuntimeTarget::Docker
        } else {
            gwt_agent::LaunchRuntimeTarget::Host
        };
        let docker_service = context
            .docker_context
            .as_ref()
            .and_then(|ctx| ctx.suggested_service.clone());
        let docker_lifecycle_intent =
            default_docker_lifecycle_intent(context.docker_service_status);
        let has_quick_start = !quick_start_entries.is_empty() || !context.live_sessions.is_empty();
        let step = if has_quick_start {
            LaunchWizardStep::QuickStart
        } else {
            LaunchWizardStep::BranchAction
        };

        let mut state = Self {
            context,
            step,
            selected: 0,
            detected_agents: agent_options,
            quick_start_entries,
            is_new_branch: false,
            base_branch_name: None,
            agent_id: String::new(),
            model: String::new(),
            reasoning: String::new(),
            version: String::new(),
            mode: "normal".to_string(),
            resume_session_id: None,
            runtime_target,
            docker_service,
            docker_lifecycle_intent,
            skip_permissions: false,
            codex_fast_mode: false,
            branch_name: String::new(),
            completion: None,
            error: None,
        };
        state.branch_name = state.context.normalized_branch_name.clone();
        state.sync_selected_agent_options();
        state.selected = step_default_selection(state.step, &state);
        state
    }

    pub fn open(context: LaunchWizardContext, sessions_dir: &Path, cache_path: &Path) -> Self {
        let agent_options = build_builtin_agent_options(
            gwt_agent::AgentDetector::detect_all(),
            &gwt_agent::VersionCache::load(cache_path),
        );
        let quick_start_entries = load_quick_start_entries(
            &context.quick_start_root,
            sessions_dir,
            &context.normalized_branch_name,
        );
        Self::open_with(context, agent_options, quick_start_entries)
    }

    pub fn view(&self) -> LaunchWizardView {
        LaunchWizardView {
            step: self.step,
            title: self.step_title(),
            branch_name: self.branch_name.clone(),
            selected: self.selected,
            step_index: self.visible_step_index(),
            step_count: self.visible_step_count(),
            can_go_back: prev_step(self.step, self).is_some(),
            options: self.current_options(),
            input_value: (self.step == LaunchWizardStep::BranchNameInput)
                .then(|| self.branch_name.clone()),
            input_label: (self.step == LaunchWizardStep::BranchNameInput)
                .then(|| "Branch name".to_string()),
            input_placeholder: (self.step == LaunchWizardStep::BranchNameInput)
                .then(|| "feature/my-work".to_string()),
            error: self.error.clone(),
        }
    }

    pub fn apply(&mut self, action: LaunchWizardAction) {
        self.error = None;

        match action {
            LaunchWizardAction::Cancel => {
                self.completion = Some(LaunchWizardCompletion::Cancelled);
            }
            LaunchWizardAction::Back => {
                if let Some(prev) = prev_step(self.step, self) {
                    self.step = prev;
                    self.selected = step_default_selection(prev, self);
                } else {
                    self.completion = Some(LaunchWizardCompletion::Cancelled);
                }
            }
            LaunchWizardAction::SubmitText { value } => {
                if self.step != LaunchWizardStep::BranchNameInput {
                    return;
                }
                let trimmed = value.trim();
                if trimmed.is_empty() {
                    self.error = Some("Branch name is required".to_string());
                    return;
                }
                self.branch_name = trimmed.to_string();
                self.advance_after_current_step();
            }
            LaunchWizardAction::Select { index } => {
                let max_index = self.current_options().len().saturating_sub(1);
                self.selected = index.min(max_index);
                self.apply_selection();
                if self.completion.is_none() {
                    self.advance_after_current_step();
                }
            }
        }
    }

    pub fn build_launch_config(&self) -> Result<gwt_agent::LaunchConfig, String> {
        let agent_id = agent_id_from_key(&self.agent_id);
        let mut builder = gwt_agent::AgentLaunchBuilder::new(agent_id.clone());

        if !self.is_new_branch {
            if let Some(worktree_path) = &self.context.worktree_path {
                builder = builder.working_dir(worktree_path.clone());
            }
        }

        if !self.branch_name.is_empty() {
            builder = builder.branch(self.branch_name.clone());
        }

        if self.is_new_branch {
            builder = builder.base_branch(
                self.base_branch_name
                    .clone()
                    .unwrap_or_else(|| DEFAULT_NEW_BRANCH_BASE_BRANCH.to_string()),
            );
        }

        if is_explicit_model_selection(&self.model) {
            builder = builder.model(self.model.clone());
        }

        if !self.version.is_empty() {
            builder = builder.version(self.version.clone());
        }

        if let Some(reasoning_level) = self.reasoning_level_for_launch() {
            builder = builder.reasoning_level(reasoning_level.to_string());
        }

        if self.skip_permissions {
            builder = builder.skip_permissions(true);
        }

        if self.agent_is_codex() && self.codex_fast_mode {
            builder = builder.fast_mode(true);
        }

        builder = builder.runtime_target(self.runtime_target);
        if let Some(docker_service) = self.docker_service.as_deref() {
            builder = builder.docker_service(docker_service.to_string());
        }
        builder = builder.docker_lifecycle_intent(self.docker_lifecycle_intent);
        builder = match self.mode.as_str() {
            "continue" => builder.session_mode(gwt_agent::SessionMode::Continue),
            "resume" if self.resume_session_id.is_some() => builder
                .session_mode(gwt_agent::SessionMode::Resume)
                .resume_session_id(self.resume_session_id.clone().expect("resume session id")),
            "resume" => builder.session_mode(gwt_agent::SessionMode::Continue),
            _ => builder.session_mode(gwt_agent::SessionMode::Normal),
        };

        let mut config = builder.build();
        if !self.version.is_empty() {
            config.tool_version = Some(self.version.clone());
        }
        if let Some(reasoning_level) = self.reasoning_level_for_launch() {
            config.reasoning_level = Some(reasoning_level.to_string());
        }
        Ok(config)
    }

    fn advance_after_current_step(&mut self) {
        if self.completion.is_some() {
            return;
        }

        if let Some(next) = next_step(self.step, self) {
            self.step = next;
            self.selected = step_default_selection(next, self);
            return;
        }

        match self.build_launch_config() {
            Ok(config) => {
                self.completion = Some(LaunchWizardCompletion::Launch(Box::new(config)));
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
    }

    fn apply_selection(&mut self) {
        match self.step {
            LaunchWizardStep::QuickStart => match self.selected_quick_start_action() {
                QuickStartAction::ResumeWithPrevious | QuickStartAction::StartNewWithPrevious => {
                    self.apply_quick_start_selection();
                    self.sync_docker_lifecycle_default();
                }
                QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => {}
            },
            LaunchWizardStep::FocusExistingSession => {
                if let Some(entry) = self.context.live_sessions.get(self.selected) {
                    self.completion = Some(LaunchWizardCompletion::FocusWindow {
                        window_id: entry.window_id.clone(),
                    });
                } else {
                    self.error = Some("No running session is available".to_string());
                }
            }
            LaunchWizardStep::BranchAction => {
                if self.selected == 0 {
                    self.is_new_branch = false;
                    self.base_branch_name = None;
                    self.branch_name = self.context.normalized_branch_name.clone();
                } else {
                    self.is_new_branch = true;
                    self.base_branch_name = Some(self.context.normalized_branch_name.clone());
                    self.branch_name.clear();
                }
            }
            LaunchWizardStep::BranchTypeSelect => {
                if let Some(prefix) = BRANCH_TYPE_PREFIXES.get(self.selected) {
                    let seed = if self.branch_name.is_empty() {
                        (*prefix).to_string()
                    } else {
                        self.branch_name.clone()
                    };
                    self.branch_name = apply_branch_prefix(&seed, prefix);
                }
            }
            LaunchWizardStep::AgentSelect => {
                if let Some(agent) = self.detected_agents.get(self.selected) {
                    self.agent_id = agent.id.clone();
                }
                self.sync_selected_agent_options();
            }
            LaunchWizardStep::ModelSelect => {
                if let Some(model) =
                    current_model_options(self.effective_agent_id()).get(self.selected)
                {
                    self.model = model.to_string();
                }
                self.sync_reasoning_state();
            }
            LaunchWizardStep::ReasoningLevel => {
                if let Some(option) = self.current_reasoning_options().get(self.selected) {
                    self.reasoning = option.stored_value.to_string();
                }
            }
            LaunchWizardStep::RuntimeTarget => {
                self.runtime_target = if self.selected == 0 {
                    gwt_agent::LaunchRuntimeTarget::Host
                } else {
                    gwt_agent::LaunchRuntimeTarget::Docker
                };
                if self.runtime_target == gwt_agent::LaunchRuntimeTarget::Host {
                    self.docker_service = None;
                } else if self.docker_service.is_none() {
                    self.docker_service = self.preferred_docker_service().map(str::to_string);
                }
                self.sync_docker_lifecycle_default();
            }
            LaunchWizardStep::DockerServiceSelect => {
                if let Some(service) = self.docker_service_options().get(self.selected) {
                    self.docker_service = Some(service.clone());
                }
                self.sync_docker_lifecycle_default();
            }
            LaunchWizardStep::DockerLifecycle => {
                if let Some(option) = self.docker_lifecycle_options().get(self.selected) {
                    self.docker_lifecycle_intent = option.intent;
                }
            }
            LaunchWizardStep::VersionSelect => {
                if let Some(option) = self.current_version_options().get(self.selected) {
                    self.version = option.value.clone();
                }
            }
            LaunchWizardStep::ExecutionMode => {
                if let Some(option) = EXECUTION_MODE_OPTIONS.get(self.selected) {
                    self.mode = option.value.to_string();
                }
            }
            LaunchWizardStep::SkipPermissions => {
                self.skip_permissions = self.selected == 0;
            }
            LaunchWizardStep::CodexFastMode => {
                self.codex_fast_mode = self.selected == 0;
            }
            LaunchWizardStep::BranchNameInput => {}
        }
    }

    fn sync_selected_agent_options(&mut self) {
        let Some(agent) = self.selected_agent().cloned() else {
            return;
        };

        if self.agent_id.is_empty() {
            self.agent_id = agent.id.clone();
        }

        let models = current_model_options(&agent.id);
        if models.is_empty() {
            self.model.clear();
        } else if self.model.is_empty() || !models.iter().any(|model| model == &self.model) {
            self.model = models[0].to_string();
        }

        let version_options = self.current_version_options_for(&agent);
        if version_options.is_empty() {
            self.version.clear();
        } else if self.version.is_empty()
            || !version_options
                .iter()
                .any(|option| option.value == self.version)
        {
            self.version = if agent_has_npm_package(&agent.id) {
                "latest".to_string()
            } else {
                "installed".to_string()
            };
        }

        if !self.agent_is_codex() {
            self.codex_fast_mode = false;
        }
        self.sync_reasoning_state();
    }

    fn sync_reasoning_state(&mut self) {
        let options = self.current_reasoning_options();
        if options.is_empty() {
            self.reasoning.clear();
            return;
        }
        if self.reasoning.is_empty()
            || !options
                .iter()
                .any(|option| option.stored_value == self.reasoning)
        {
            self.reasoning = options
                .iter()
                .find(|option| option.is_default)
                .map(|option| option.stored_value.to_string())
                .unwrap_or_default();
        }
    }

    fn sync_docker_lifecycle_default(&mut self) {
        let supported = self
            .docker_lifecycle_options()
            .iter()
            .any(|option| option.intent == self.docker_lifecycle_intent);
        if !supported {
            self.docker_lifecycle_intent =
                default_docker_lifecycle_intent(self.context.docker_service_status);
        }
    }

    fn reasoning_level_for_launch(&self) -> Option<&str> {
        match self.effective_agent_id() {
            "codex" if !self.reasoning.is_empty() => Some(self.reasoning.as_str()),
            "claude"
                if !self.reasoning.is_empty()
                    && matches!(
                        self.model.as_str(),
                        "Default (Opus 4.6)" | "opus" | "sonnet"
                    ) =>
            {
                Some(self.reasoning.as_str())
            }
            _ => None,
        }
    }

    fn step_title(&self) -> String {
        match self.step {
            LaunchWizardStep::ReasoningLevel => {
                if self.agent_is_codex() {
                    "Select Reasoning Level".to_string()
                } else if self.effective_agent_id() == "claude" {
                    "Select Effort Level".to_string()
                } else {
                    self.step.title().to_string()
                }
            }
            _ => self.step.title().to_string(),
        }
    }

    fn selected_agent(&self) -> Option<&AgentOption> {
        if self.step == LaunchWizardStep::AgentSelect {
            return self.detected_agents.get(self.selected);
        }
        if self.agent_id.is_empty() {
            self.detected_agents.first()
        } else {
            self.detected_agents
                .iter()
                .find(|agent| agent.id == self.agent_id)
        }
    }

    fn effective_agent_id(&self) -> &str {
        self.selected_agent()
            .map(|agent| agent.id.as_str())
            .unwrap_or(self.agent_id.as_str())
    }

    fn agent_is_codex(&self) -> bool {
        self.effective_agent_id() == "codex"
    }

    fn agent_has_models(&self) -> bool {
        matches!(self.effective_agent_id(), "claude" | "codex" | "gemini")
    }

    fn agent_uses_reasoning_step(&self) -> bool {
        if self.agent_is_codex() {
            return true;
        }
        self.effective_agent_id() == "claude"
            && matches!(
                self.model.as_str(),
                "Default (Opus 4.6)" | "opus" | "sonnet"
            )
    }

    fn has_docker_workflow(&self) -> bool {
        self.context.docker_context.is_some()
    }

    fn docker_service_prompt_required(&self) -> bool {
        self.context
            .docker_context
            .as_ref()
            .is_some_and(|ctx| ctx.services.len() > 1)
    }

    fn preferred_docker_service(&self) -> Option<&str> {
        self.docker_service.as_deref().or_else(|| {
            self.context
                .docker_context
                .as_ref()
                .and_then(|ctx| ctx.suggested_service.as_deref())
        })
    }

    fn docker_service_options(&self) -> Vec<String> {
        self.context
            .docker_context
            .as_ref()
            .map(|ctx| ctx.services.clone())
            .unwrap_or_default()
    }

    fn docker_lifecycle_options(&self) -> &'static [DockerLifecycleOption] {
        match self.context.docker_service_status {
            gwt_docker::ComposeServiceStatus::Running => &[
                DockerLifecycleOption {
                    label: "Connect only",
                    description: "Reuse the running Docker service",
                    intent: gwt_agent::DockerLifecycleIntent::Connect,
                },
                DockerLifecycleOption {
                    label: "Restart then launch",
                    description: "Restart the Docker service before launching",
                    intent: gwt_agent::DockerLifecycleIntent::Restart,
                },
                DockerLifecycleOption {
                    label: "Recreate then launch",
                    description: "Force-recreate the Docker service before launching",
                    intent: gwt_agent::DockerLifecycleIntent::Recreate,
                },
            ],
            gwt_docker::ComposeServiceStatus::Stopped
            | gwt_docker::ComposeServiceStatus::Exited => &[
                DockerLifecycleOption {
                    label: "Start then launch",
                    description: "Start the existing Docker service",
                    intent: gwt_agent::DockerLifecycleIntent::Start,
                },
                DockerLifecycleOption {
                    label: "Recreate then launch",
                    description: "Force-recreate the Docker service before launching",
                    intent: gwt_agent::DockerLifecycleIntent::Recreate,
                },
            ],
            gwt_docker::ComposeServiceStatus::NotFound => &[DockerLifecycleOption {
                label: "Create and start then launch",
                description: "Create the Docker service and launch into it",
                intent: gwt_agent::DockerLifecycleIntent::CreateAndStart,
            }],
        }
    }

    fn current_version_options_for(&self, agent: &AgentOption) -> Vec<gwt_agent::VersionOption> {
        gwt_agent::build_version_options(
            agent.available,
            agent.installed_version.as_deref(),
            agent_has_npm_package(&agent.id),
            &agent.versions,
        )
    }

    fn current_version_options(&self) -> Vec<gwt_agent::VersionOption> {
        self.selected_agent()
            .map(|agent| self.current_version_options_for(agent))
            .unwrap_or_default()
    }

    fn current_reasoning_options(&self) -> &'static [ReasoningDisplayOption] {
        if self.agent_is_codex() {
            &CODEX_REASONING_OPTIONS
        } else if self.effective_agent_id() == "claude"
            && matches!(self.model.as_str(), "Default (Opus 4.6)" | "opus")
        {
            &CLAUDE_OPUS_REASONING_OPTIONS
        } else if self.effective_agent_id() == "claude" && self.model == "sonnet" {
            &CLAUDE_SONNET_REASONING_OPTIONS
        } else {
            &[]
        }
    }

    fn selected_quick_start_action(&self) -> QuickStartAction {
        let choose_different_index = self.quick_start_choose_different_index();
        if self.selected >= choose_different_index {
            QuickStartAction::ChooseDifferent
        } else if self.selected < self.quick_start_entries.len() * 2
            && self.selected.is_multiple_of(2)
        {
            QuickStartAction::ResumeWithPrevious
        } else if self.selected < self.quick_start_entries.len() * 2 {
            QuickStartAction::StartNewWithPrevious
        } else {
            QuickStartAction::FocusExistingSession
        }
    }

    fn selected_quick_start_entry(&self) -> Option<&QuickStartEntry> {
        if self.quick_start_entries.is_empty()
            || self.selected >= self.quick_start_entries.len() * 2
        {
            None
        } else {
            self.quick_start_entries.get(self.selected / 2)
        }
    }

    fn quick_start_choose_different_index(&self) -> usize {
        self.quick_start_entries.len() * 2 + usize::from(!self.context.live_sessions.is_empty())
    }

    fn apply_quick_start_selection(&mut self) {
        let Some(entry) = self.selected_quick_start_entry().cloned() else {
            return;
        };

        self.agent_id = entry.agent_id.clone();
        if let Some(index) = self
            .detected_agents
            .iter()
            .position(|agent| agent.id == entry.agent_id)
        {
            self.selected = index;
        }
        self.sync_selected_agent_options();

        if let Some(model) = entry.model {
            self.model = model;
        }
        if let Some(reasoning) = entry.reasoning {
            self.reasoning = reasoning;
        }
        if let Some(version) = entry.version {
            self.version = version;
        }
        self.skip_permissions = entry.skip_permissions;
        self.codex_fast_mode = entry.codex_fast_mode && self.agent_is_codex();
        self.runtime_target = entry.runtime_target;
        self.docker_service = entry.docker_service.clone();
        self.docker_lifecycle_intent = entry.docker_lifecycle_intent;

        match self.selected_quick_start_action() {
            QuickStartAction::ResumeWithPrevious => {
                if let Some(resume_session_id) = entry.resume_session_id {
                    self.mode = "resume".to_string();
                    self.resume_session_id = Some(resume_session_id);
                } else {
                    self.mode = "continue".to_string();
                    self.resume_session_id = None;
                }
            }
            QuickStartAction::StartNewWithPrevious => {
                self.mode = "normal".to_string();
                self.resume_session_id = None;
            }
            QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => {}
        }
    }

    fn visible_step_count(&self) -> usize {
        let mut count = 0;
        let mut step = Some(self.flow_start_step());
        while let Some(current) = step {
            count += 1;
            step = next_step(current, self);
        }
        count
    }

    fn visible_step_index(&self) -> usize {
        let mut index = 0;
        let mut step = Some(self.flow_start_step());
        while let Some(current) = step {
            index += 1;
            if current == self.step {
                return index;
            }
            step = next_step(current, self);
        }
        index
    }

    fn flow_start_step(&self) -> LaunchWizardStep {
        if !self.quick_start_entries.is_empty() || !self.context.live_sessions.is_empty() {
            LaunchWizardStep::QuickStart
        } else {
            LaunchWizardStep::BranchAction
        }
    }

    fn current_options(&self) -> Vec<LaunchWizardOptionView> {
        match self.step {
            LaunchWizardStep::QuickStart => {
                let mut options = Vec::new();
                for entry in &self.quick_start_entries {
                    let summary = quick_start_summary(entry);
                    options.push(LaunchWizardOptionView {
                        label: format!("Resume {}", entry.tool_label),
                        description: Some(summary.clone()),
                    });
                    options.push(LaunchWizardOptionView {
                        label: format!("Start new with {}", entry.tool_label),
                        description: Some(summary),
                    });
                }
                if !self.context.live_sessions.is_empty() {
                    options.push(LaunchWizardOptionView {
                        label: "Focus existing session".to_string(),
                        description: Some("Jump to a running window on this branch".to_string()),
                    });
                }
                options.push(LaunchWizardOptionView {
                    label: "Choose different".to_string(),
                    description: Some("Open the full launch wizard".to_string()),
                });
                options
            }
            LaunchWizardStep::FocusExistingSession => self
                .context
                .live_sessions
                .iter()
                .map(|entry| LaunchWizardOptionView {
                    label: entry.name.clone(),
                    description: entry.detail.clone(),
                })
                .collect(),
            LaunchWizardStep::BranchAction => vec![
                LaunchWizardOptionView {
                    label: "Use selected branch".to_string(),
                    description: Some("Launch on the selected branch".to_string()),
                },
                LaunchWizardOptionView {
                    label: "Create new from selected".to_string(),
                    description: Some(
                        "Create a new branch based on the selected branch".to_string(),
                    ),
                },
            ],
            LaunchWizardStep::BranchTypeSelect => BRANCH_TYPE_PREFIXES
                .iter()
                .map(|prefix| LaunchWizardOptionView {
                    label: (*prefix).to_string(),
                    description: Some(format!(
                        "Use {} as the branch prefix",
                        prefix.trim_end_matches('/')
                    )),
                })
                .collect(),
            LaunchWizardStep::AgentSelect => self
                .detected_agents
                .iter()
                .map(|agent| LaunchWizardOptionView {
                    label: agent.name.clone(),
                    description: Some(agent_description(agent)),
                })
                .collect(),
            LaunchWizardStep::ModelSelect => model_display_options(self.effective_agent_id())
                .iter()
                .map(|option| LaunchWizardOptionView {
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                })
                .collect(),
            LaunchWizardStep::ReasoningLevel => self
                .current_reasoning_options()
                .iter()
                .map(|option| LaunchWizardOptionView {
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                })
                .collect(),
            LaunchWizardStep::RuntimeTarget => RUNTIME_TARGET_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                })
                .collect(),
            LaunchWizardStep::DockerServiceSelect => self
                .docker_service_options()
                .into_iter()
                .map(|service| LaunchWizardOptionView {
                    label: service,
                    description: Some("Docker Compose service".to_string()),
                })
                .collect(),
            LaunchWizardStep::DockerLifecycle => self
                .docker_lifecycle_options()
                .iter()
                .map(|option| LaunchWizardOptionView {
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                })
                .collect(),
            LaunchWizardStep::VersionSelect => self
                .current_version_options()
                .into_iter()
                .map(|option| LaunchWizardOptionView {
                    label: option.label,
                    description: Some("Tool version".to_string()),
                })
                .collect(),
            LaunchWizardStep::ExecutionMode => EXECUTION_MODE_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                })
                .collect(),
            LaunchWizardStep::SkipPermissions => YES_NO_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                })
                .collect(),
            LaunchWizardStep::CodexFastMode => FAST_MODE_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                })
                .collect(),
            LaunchWizardStep::BranchNameInput => Vec::new(),
        }
    }
}

#[derive(Clone, Copy)]
struct ModelDisplayOption {
    label: &'static str,
    description: &'static str,
}

#[derive(Clone, Copy)]
struct ReasoningDisplayOption {
    label: &'static str,
    stored_value: &'static str,
    description: &'static str,
    is_default: bool,
}

#[derive(Clone, Copy)]
struct ChoiceOption {
    label: &'static str,
    description: &'static str,
}

#[derive(Clone, Copy)]
struct ExecutionModeOption {
    label: &'static str,
    description: &'static str,
    value: &'static str,
}

#[derive(Clone, Copy)]
struct DockerLifecycleOption {
    label: &'static str,
    description: &'static str,
    intent: gwt_agent::DockerLifecycleIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuickStartAction {
    ResumeWithPrevious,
    StartNewWithPrevious,
    FocusExistingSession,
    ChooseDifferent,
}

const CLAUDE_MODEL_OPTIONS: [ModelDisplayOption; 4] = [
    ModelDisplayOption {
        label: "Default (Opus 4.6)",
        description: "Most capable for complex work",
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

const CODEX_MODEL_OPTIONS: [ModelDisplayOption; 9] = [
    ModelDisplayOption {
        label: "Default (Auto)",
        description: "Use Codex default model",
    },
    ModelDisplayOption {
        label: "gpt-5.4",
        description: "Latest frontier agentic coding model",
    },
    ModelDisplayOption {
        label: "gpt-5.4-mini",
        description: "Smaller frontier agentic coding model",
    },
    ModelDisplayOption {
        label: "gpt-5.3-codex",
        description: "Frontier Codex-optimized coding model",
    },
    ModelDisplayOption {
        label: "gpt-5.3-codex-spark",
        description: "Ultra-fast coding model",
    },
    ModelDisplayOption {
        label: "gpt-5.2-codex",
        description: "Frontier agentic coding model",
    },
    ModelDisplayOption {
        label: "gpt-5.2",
        description: "Optimized for professional work",
    },
    ModelDisplayOption {
        label: "gpt-5.1-codex-max",
        description: "Deep and fast reasoning",
    },
    ModelDisplayOption {
        label: "gpt-5.1-codex-mini",
        description: "Cheaper and faster codex option",
    },
];

const GEMINI_MODEL_OPTIONS: [ModelDisplayOption; 6] = [
    ModelDisplayOption {
        label: "Default (Auto)",
        description: "Use Gemini default model",
    },
    ModelDisplayOption {
        label: "gemini-3-pro-preview",
        description: "Preview pro model",
    },
    ModelDisplayOption {
        label: "gemini-3-flash-preview",
        description: "Preview flash model",
    },
    ModelDisplayOption {
        label: "gemini-2.5-pro",
        description: "Stable pro model",
    },
    ModelDisplayOption {
        label: "gemini-2.5-flash",
        description: "Balanced speed and reasoning",
    },
    ModelDisplayOption {
        label: "gemini-2.5-flash-lite",
        description: "Fastest Gemini option",
    },
];

const CLAUDE_OPUS_REASONING_OPTIONS: [ReasoningDisplayOption; 5] = [
    ReasoningDisplayOption {
        label: "Auto",
        stored_value: "auto",
        description: "Let the model choose the effort",
        is_default: false,
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
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Deeper reasoning for complex work",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Max",
        stored_value: "max",
        description: "Deepest reasoning",
        is_default: false,
    },
];

const CLAUDE_SONNET_REASONING_OPTIONS: [ReasoningDisplayOption; 4] = [
    CLAUDE_OPUS_REASONING_OPTIONS[0],
    CLAUDE_OPUS_REASONING_OPTIONS[1],
    CLAUDE_OPUS_REASONING_OPTIONS[2],
    CLAUDE_OPUS_REASONING_OPTIONS[3],
];

const CODEX_REASONING_OPTIONS: [ReasoningDisplayOption; 4] = [
    ReasoningDisplayOption {
        label: "Low",
        stored_value: "low",
        description: "Fast responses with lighter reasoning",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Medium",
        stored_value: "medium",
        description: "Balances speed and reasoning depth",
        is_default: true,
    },
    ReasoningDisplayOption {
        label: "High",
        stored_value: "high",
        description: "Greater reasoning depth",
        is_default: false,
    },
    ReasoningDisplayOption {
        label: "Extra high",
        stored_value: "xhigh",
        description: "Maximum reasoning depth",
        is_default: false,
    },
];

const EXECUTION_MODE_OPTIONS: [ExecutionModeOption; 3] = [
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
        description: "Resume the selected session metadata",
        value: "resume",
    },
];

const RUNTIME_TARGET_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "Host",
        description: "Run directly on the host",
    },
    ChoiceOption {
        label: "Docker",
        description: "Run inside the detected Docker service",
    },
];

const YES_NO_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "Yes",
        description: "Skip permission prompts",
    },
    ChoiceOption {
        label: "No",
        description: "Show permission prompts",
    },
];

const FAST_MODE_OPTIONS: [ChoiceOption; 2] = [
    ChoiceOption {
        label: "On",
        description: "Use Codex fast service tier",
    },
    ChoiceOption {
        label: "Off",
        description: "Use the standard service tier",
    },
];

fn default_docker_lifecycle_intent(
    status: gwt_docker::ComposeServiceStatus,
) -> gwt_agent::DockerLifecycleIntent {
    match status {
        gwt_docker::ComposeServiceStatus::Running => gwt_agent::DockerLifecycleIntent::Connect,
        gwt_docker::ComposeServiceStatus::Stopped | gwt_docker::ComposeServiceStatus::Exited => {
            gwt_agent::DockerLifecycleIntent::Start
        }
        gwt_docker::ComposeServiceStatus::NotFound => {
            gwt_agent::DockerLifecycleIntent::CreateAndStart
        }
    }
}

fn next_step(current: LaunchWizardStep, state: &LaunchWizardState) -> Option<LaunchWizardStep> {
    match current {
        LaunchWizardStep::QuickStart => match state.selected_quick_start_action() {
            QuickStartAction::ChooseDifferent => Some(LaunchWizardStep::BranchAction),
            QuickStartAction::FocusExistingSession => Some(LaunchWizardStep::FocusExistingSession),
            QuickStartAction::ResumeWithPrevious | QuickStartAction::StartNewWithPrevious => {
                Some(LaunchWizardStep::SkipPermissions)
            }
        },
        LaunchWizardStep::FocusExistingSession => None,
        LaunchWizardStep::BranchAction => {
            if state.selected == 0 {
                Some(LaunchWizardStep::AgentSelect)
            } else {
                Some(LaunchWizardStep::BranchTypeSelect)
            }
        }
        LaunchWizardStep::BranchTypeSelect => Some(LaunchWizardStep::BranchNameInput),
        LaunchWizardStep::BranchNameInput => Some(LaunchWizardStep::AgentSelect),
        LaunchWizardStep::AgentSelect => {
            if state.agent_has_models() {
                Some(LaunchWizardStep::ModelSelect)
            } else if state.has_docker_workflow() {
                Some(LaunchWizardStep::RuntimeTarget)
            } else if agent_has_npm_package(state.effective_agent_id()) {
                Some(LaunchWizardStep::VersionSelect)
            } else {
                Some(LaunchWizardStep::ExecutionMode)
            }
        }
        LaunchWizardStep::ModelSelect => {
            if state.agent_uses_reasoning_step() {
                Some(LaunchWizardStep::ReasoningLevel)
            } else if state.has_docker_workflow() {
                Some(LaunchWizardStep::RuntimeTarget)
            } else if agent_has_npm_package(state.effective_agent_id()) {
                Some(LaunchWizardStep::VersionSelect)
            } else {
                Some(LaunchWizardStep::ExecutionMode)
            }
        }
        LaunchWizardStep::ReasoningLevel => {
            if state.has_docker_workflow() {
                Some(LaunchWizardStep::RuntimeTarget)
            } else if agent_has_npm_package(state.effective_agent_id()) {
                Some(LaunchWizardStep::VersionSelect)
            } else {
                Some(LaunchWizardStep::ExecutionMode)
            }
        }
        LaunchWizardStep::RuntimeTarget => {
            if state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker
                && state.docker_service_prompt_required()
            {
                Some(LaunchWizardStep::DockerServiceSelect)
            } else if state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                Some(LaunchWizardStep::DockerLifecycle)
            } else if agent_has_npm_package(state.effective_agent_id()) {
                Some(LaunchWizardStep::VersionSelect)
            } else {
                Some(LaunchWizardStep::ExecutionMode)
            }
        }
        LaunchWizardStep::DockerServiceSelect => Some(LaunchWizardStep::DockerLifecycle),
        LaunchWizardStep::DockerLifecycle => {
            if agent_has_npm_package(state.effective_agent_id()) {
                Some(LaunchWizardStep::VersionSelect)
            } else {
                Some(LaunchWizardStep::ExecutionMode)
            }
        }
        LaunchWizardStep::VersionSelect => Some(LaunchWizardStep::ExecutionMode),
        LaunchWizardStep::ExecutionMode => Some(LaunchWizardStep::SkipPermissions),
        LaunchWizardStep::SkipPermissions => {
            if state.agent_is_codex() {
                Some(LaunchWizardStep::CodexFastMode)
            } else {
                None
            }
        }
        LaunchWizardStep::CodexFastMode => None,
    }
}

fn prev_step(current: LaunchWizardStep, state: &LaunchWizardState) -> Option<LaunchWizardStep> {
    match current {
        LaunchWizardStep::QuickStart => None,
        LaunchWizardStep::FocusExistingSession => Some(LaunchWizardStep::QuickStart),
        LaunchWizardStep::BranchAction => {
            if !state.quick_start_entries.is_empty() || !state.context.live_sessions.is_empty() {
                Some(LaunchWizardStep::QuickStart)
            } else {
                None
            }
        }
        LaunchWizardStep::BranchTypeSelect => Some(LaunchWizardStep::BranchAction),
        LaunchWizardStep::BranchNameInput => Some(LaunchWizardStep::BranchTypeSelect),
        LaunchWizardStep::AgentSelect => {
            if state.is_new_branch {
                Some(LaunchWizardStep::BranchNameInput)
            } else {
                Some(LaunchWizardStep::BranchAction)
            }
        }
        LaunchWizardStep::ModelSelect => Some(LaunchWizardStep::AgentSelect),
        LaunchWizardStep::ReasoningLevel => Some(LaunchWizardStep::ModelSelect),
        LaunchWizardStep::RuntimeTarget => {
            if state.agent_uses_reasoning_step() {
                Some(LaunchWizardStep::ReasoningLevel)
            } else if state.agent_has_models() {
                Some(LaunchWizardStep::ModelSelect)
            } else {
                Some(LaunchWizardStep::AgentSelect)
            }
        }
        LaunchWizardStep::DockerServiceSelect => Some(LaunchWizardStep::RuntimeTarget),
        LaunchWizardStep::DockerLifecycle => {
            if state.docker_service_prompt_required() {
                Some(LaunchWizardStep::DockerServiceSelect)
            } else {
                Some(LaunchWizardStep::RuntimeTarget)
            }
        }
        LaunchWizardStep::VersionSelect => {
            if state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                Some(LaunchWizardStep::DockerLifecycle)
            } else if state.has_docker_workflow() {
                Some(LaunchWizardStep::RuntimeTarget)
            } else if state.agent_uses_reasoning_step() {
                Some(LaunchWizardStep::ReasoningLevel)
            } else if state.agent_has_models() {
                Some(LaunchWizardStep::ModelSelect)
            } else {
                Some(LaunchWizardStep::AgentSelect)
            }
        }
        LaunchWizardStep::ExecutionMode => {
            if state.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                Some(LaunchWizardStep::DockerLifecycle)
            } else if agent_has_npm_package(state.effective_agent_id()) {
                Some(LaunchWizardStep::VersionSelect)
            } else if state.has_docker_workflow() {
                Some(LaunchWizardStep::RuntimeTarget)
            } else if state.agent_uses_reasoning_step() {
                Some(LaunchWizardStep::ReasoningLevel)
            } else if state.agent_has_models() {
                Some(LaunchWizardStep::ModelSelect)
            } else {
                Some(LaunchWizardStep::AgentSelect)
            }
        }
        LaunchWizardStep::SkipPermissions => Some(LaunchWizardStep::ExecutionMode),
        LaunchWizardStep::CodexFastMode => Some(LaunchWizardStep::SkipPermissions),
    }
}

fn step_default_selection(step: LaunchWizardStep, state: &LaunchWizardState) -> usize {
    match step {
        LaunchWizardStep::QuickStart => 0,
        LaunchWizardStep::FocusExistingSession => 0,
        LaunchWizardStep::BranchAction => 0,
        LaunchWizardStep::BranchTypeSelect => 0,
        LaunchWizardStep::BranchNameInput => 0,
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
        LaunchWizardStep::ExecutionMode => EXECUTION_MODE_OPTIONS
            .iter()
            .position(|option| option.value == state.mode)
            .unwrap_or(0),
        LaunchWizardStep::SkipPermissions => usize::from(!state.skip_permissions),
        LaunchWizardStep::CodexFastMode => usize::from(!state.codex_fast_mode),
    }
}

fn current_model_options(agent_id: &str) -> Vec<&'static str> {
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

fn model_display_options(agent_id: &str) -> &'static [ModelDisplayOption] {
    match agent_id {
        "claude" => &CLAUDE_MODEL_OPTIONS,
        "codex" => &CODEX_MODEL_OPTIONS,
        "gemini" => &GEMINI_MODEL_OPTIONS,
        _ => &[],
    }
}

fn quick_start_summary(entry: &QuickStartEntry) -> String {
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

fn apply_branch_prefix(seed: &str, prefix: &str) -> String {
    let trimmed = seed.trim();
    let suffix = BRANCH_TYPE_PREFIXES
        .iter()
        .find_map(|known| trimmed.strip_prefix(known))
        .unwrap_or(trimmed)
        .trim_matches('/');
    if suffix.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix}{suffix}")
    }
}

fn is_explicit_model_selection(model: &str) -> bool {
    !model.is_empty() && !model.starts_with("Default")
}

fn agent_has_npm_package(agent_id: &str) -> bool {
    matches!(agent_id, "claude" | "codex" | "gemini")
}

fn agent_id_from_key(agent_id: &str) -> gwt_agent::AgentId {
    match agent_id {
        "claude" => gwt_agent::AgentId::ClaudeCode,
        "codex" => gwt_agent::AgentId::Codex,
        "gemini" => gwt_agent::AgentId::Gemini,
        "gh" => gwt_agent::AgentId::Copilot,
        other => gwt_agent::AgentId::Custom(other.to_string()),
    }
}

fn agent_description(agent: &AgentOption) -> String {
    let availability = if agent.available {
        "Installed"
    } else {
        "Not installed"
    };
    match agent.installed_version.as_deref() {
        Some(version) => format!("{availability} · {version}"),
        None => availability.to_string(),
    }
}

pub fn default_wizard_version_cache_path() -> PathBuf {
    gwt_core::paths::gwt_cache_dir().join("agent-versions.json")
}

pub fn build_builtin_agent_options(
    detected_agents: Vec<gwt_agent::DetectedAgent>,
    cache: &gwt_agent::VersionCache,
) -> Vec<AgentOption> {
    const BUILTIN: [gwt_agent::AgentId; 4] = [
        gwt_agent::AgentId::ClaudeCode,
        gwt_agent::AgentId::Codex,
        gwt_agent::AgentId::Gemini,
        gwt_agent::AgentId::Copilot,
    ];

    BUILTIN
        .into_iter()
        .map(|agent_id| {
            let detected = detected_agents
                .iter()
                .find(|detected| detected.agent_id == agent_id);
            AgentOption {
                id: agent_id.command().to_string(),
                name: agent_id.display_name().to_string(),
                available: detected.is_some(),
                installed_version: detected.and_then(|detected| detected.version.clone()),
                versions: cache
                    .get(&agent_id)
                    .map(|versions| versions.to_vec())
                    .unwrap_or_default(),
            }
        })
        .collect()
}

pub fn load_quick_start_entries(
    repo_path: &Path,
    sessions_dir: &Path,
    branch_name: &str,
) -> Vec<QuickStartEntry> {
    let Ok(entries) = std::fs::read_dir(sessions_dir) else {
        return Vec::new();
    };

    let mut latest_by_agent: HashMap<String, gwt_agent::Session> = HashMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("toml") {
            continue;
        }
        let Ok(session) = gwt_agent::Session::load(&path) else {
            continue;
        };
        if session.branch != branch_name || session.worktree_path != repo_path {
            continue;
        }

        let agent_key = session.agent_id.command().to_string();
        let replace = latest_by_agent
            .get(&agent_key)
            .map(|current| {
                session.updated_at > current.updated_at
                    || (session.updated_at == current.updated_at
                        && session.created_at > current.created_at)
            })
            .unwrap_or(true);
        if replace {
            latest_by_agent.insert(agent_key, session);
        }
    }

    let mut sessions = latest_by_agent.into_values().collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.created_at.cmp(&left.created_at))
    });

    sessions
        .into_iter()
        .map(|session| QuickStartEntry {
            agent_id: session.agent_id.command().to_string(),
            tool_label: session.display_name.clone(),
            model: session.model.clone(),
            reasoning: session.reasoning_level.clone(),
            version: session.tool_version.clone().or_else(|| {
                session
                    .agent_id
                    .package_name()
                    .map(|_| "installed".to_string())
            }),
            resume_session_id: session.agent_session_id.clone(),
            skip_permissions: session.skip_permissions,
            codex_fast_mode: session.codex_fast_mode,
            runtime_target: session.runtime_target,
            docker_service: session.docker_service.clone(),
            docker_lifecycle_intent: session.docker_lifecycle_intent,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    fn sample_agent_options() -> Vec<AgentOption> {
        vec![
            AgentOption {
                id: "claude".to_string(),
                name: "Claude Code".to_string(),
                available: true,
                installed_version: Some("1.0.0".to_string()),
                versions: vec!["0.9.0".to_string(), "1.0.0".to_string()],
            },
            AgentOption {
                id: "codex".to_string(),
                name: "Codex".to_string(),
                available: true,
                installed_version: Some("0.110.0".to_string()),
                versions: vec!["0.109.0".to_string(), "0.110.0".to_string()],
            },
        ]
    }

    fn branch(name: &str) -> BranchListEntry {
        BranchListEntry {
            name: name.to_string(),
            scope: crate::BranchScope::Local,
            is_head: false,
            upstream: Some(format!("origin/{name}")),
            ahead: 0,
            behind: 0,
            last_commit_date: None,
        }
    }

    fn context(branch: BranchListEntry, normalized: &str) -> LaunchWizardContext {
        LaunchWizardContext {
            selected_branch: branch,
            normalized_branch_name: normalized.to_string(),
            worktree_path: None,
            quick_start_root: PathBuf::from("/tmp/repo"),
            live_sessions: Vec::new(),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
        }
    }

    fn sample_session(
        dir: &Path,
        branch: &str,
        worktree_path: &Path,
        agent_id: gwt_agent::AgentId,
        updated_at: chrono::DateTime<Utc>,
        resume_id: &str,
    ) {
        let mut session = gwt_agent::Session::new(worktree_path, branch, agent_id);
        session.display_name = session.agent_id.display_name().to_string();
        session.agent_session_id = Some(resume_id.to_string());
        session.tool_version = Some("installed".to_string());
        session.model = Some("gpt-5.4".to_string());
        session.reasoning_level = Some("high".to_string());
        session.skip_permissions = true;
        session.codex_fast_mode = true;
        session.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        session.docker_service = Some("gwt".to_string());
        session.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;
        session.created_at = updated_at;
        session.updated_at = updated_at;
        session.last_activity_at = updated_at;
        session.save(dir).expect("save session");
    }

    #[test]
    fn open_local_branch_without_quick_start_starts_at_branch_action() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        assert_eq!(state.step, LaunchWizardStep::BranchAction);
        assert_eq!(state.branch_name, "feature/gui");
        assert!(!state.is_new_branch);
    }

    #[test]
    fn open_with_quick_start_prefers_quick_start_step() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![QuickStartEntry {
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.4".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                resume_session_id: Some("resume-1".to_string()),
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        assert_eq!(state.step, LaunchWizardStep::QuickStart);
    }

    #[test]
    fn load_quick_start_entries_prefers_latest_session_per_agent() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 9, 0, 0).unwrap(),
            "older",
        );
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 10, 0, 0).unwrap(),
            "newer",
        );

        let entries = load_quick_start_entries(&worktree, dir.path(), "feature/gui");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].agent_id, "codex");
        assert_eq!(entries[0].resume_session_id.as_deref(), Some("newer"));
        assert_eq!(entries[0].docker_service.as_deref(), Some("gwt"));
    }

    #[test]
    fn branch_action_create_new_from_selected_sets_base_branch() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        state.apply(LaunchWizardAction::Select { index: 1 });

        assert_eq!(state.step, LaunchWizardStep::BranchTypeSelect);
        assert!(state.is_new_branch);
        assert_eq!(state.base_branch_name.as_deref(), Some("feature/gui"));
    }

    #[test]
    fn build_launch_config_for_codex_resume_uses_resume_session_id() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        state.agent_id = "codex".to_string();
        state.model = "gpt-5.4".to_string();
        state.reasoning = "high".to_string();
        state.version = "0.110.0".to_string();
        state.mode = "resume".to_string();
        state.resume_session_id = Some("session-123".to_string());
        state.skip_permissions = true;
        state.codex_fast_mode = true;
        state.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        state.docker_service = Some("gwt".to_string());
        state.docker_lifecycle_intent = gwt_agent::DockerLifecycleIntent::Restart;

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
        assert_eq!(config.branch.as_deref(), Some("feature/gui"));
        assert_eq!(config.resume_session_id.as_deref(), Some("session-123"));
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
        assert_eq!(config.reasoning_level.as_deref(), Some("high"));
        assert_eq!(config.tool_version.as_deref(), Some("0.110.0"));
        assert_eq!(config.docker_service.as_deref(), Some("gwt"));
        assert!(config.skip_permissions);
        assert!(config.codex_fast_mode);
    }
}
