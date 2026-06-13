use super::*;

impl LaunchWizardState {
    fn hydrate_live_window_ids(
        context: &LaunchWizardContext,
        quick_start_entries: &mut [QuickStartEntry],
    ) {
        for entry in quick_start_entries {
            entry.live_window_id = context
                .live_sessions
                .iter()
                .find(|session| session.session_id == entry.session_id)
                .or_else(|| {
                    context
                        .live_sessions
                        .iter()
                        .find(|session| session.agent_id == entry.agent_id)
                })
                .map(|session| session.window_id.clone());
        }
    }

    fn new_with(
        context: LaunchWizardContext,
        agent_options: Vec<AgentOption>,
        mut quick_start_entries: Vec<QuickStartEntry>,
        previous_profiles: LaunchWizardPreviousProfiles,
        is_hydrating: bool,
    ) -> Self {
        Self::hydrate_live_window_ids(&context, &mut quick_start_entries);
        // SPEC-2014 FR-032..FR-035: 初期 runtime_target / docker_service / docker_lifecycle_intent は
        // open Wizard draft (= 開いた直後はまだ無い) → repo-local previous session → context default
        // の順で決定する。runtime/Docker の復元は agent map ではなく `repo_local` 経路に閉じ込め、
        // global agent preference path (apply_previous_agent_preferences) は触れない。
        let (runtime_target, docker_service, docker_lifecycle_intent) =
            resolve_initial_runtime_selection(&context, previous_profiles.repo_local());
        let has_quick_start = !quick_start_entries.is_empty() || !context.live_sessions.is_empty();
        let step = if has_quick_start {
            LaunchWizardStep::QuickStart
        } else {
            LaunchWizardStep::BranchAction
        };
        let launch_path = default_launch_path(&context, &quick_start_entries);

        let mut state = Self {
            context: context.clone(),
            wizard_mode: LaunchWizardMode::Branch,
            step,
            selected: 0,
            launch_path,
            selected_quick_start_index: (!quick_start_entries.is_empty()).then_some(0),
            selected_live_session_index: (!context.live_sessions.is_empty()).then_some(0),
            detected_agents: agent_options,
            quick_start_entries,
            previous_profiles,
            is_new_branch: false,
            base_branch_name: None,
            launch_target: LaunchTargetKind::Agent,
            agent_id: String::new(),
            agent_drafts: HashMap::new(),
            model: String::new(),
            reasoning: String::new(),
            version: String::new(),
            mode: "normal".to_string(),
            resume_session_id: None,
            runtime_target,
            windows_shell: default_windows_shell_kind(),
            docker_service,
            docker_lifecycle_intent,
            skip_permissions: false,
            codex_fast_mode: false,
            branch_name: String::new(),
            completion: None,
            error: None,
            is_hydrating,
            runtime_context_resolved: true,
            runtime_resolution_pending: false,
            runtime_resolution_message: None,
            hydration_error: None,
            linked_issue_number: context.linked_issue_number,
            start_method_selected: false,
            manual_setup_initialized: false,
            runtime_confirmed: false,
            settings_revisited: false,
            resolved_branch_name: None,
        };
        state.branch_name = state.context.normalized_branch_name.clone();
        state.sync_selected_agent_options();
        state.apply_preferred_agent_profile();
        state.sync_docker_lifecycle_default();
        state.selected = step_default_selection(state.step, &state);
        state
    }

    pub fn open_with(
        context: LaunchWizardContext,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
    ) -> Self {
        Self::new_with(
            context,
            agent_options,
            quick_start_entries,
            LaunchWizardPreviousProfiles::default(),
            false,
        )
    }

    pub fn open_with_previous_profiles(
        context: LaunchWizardContext,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
        previous_profiles: LaunchWizardPreviousProfiles,
    ) -> Self {
        Self::new_with(
            context,
            agent_options,
            quick_start_entries,
            previous_profiles,
            false,
        )
    }

    pub fn open_with_previous_profile(
        context: LaunchWizardContext,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
        previous_profile: Option<LaunchWizardPreviousProfile>,
    ) -> Self {
        Self::open_with_previous_profiles(
            context,
            agent_options,
            quick_start_entries,
            LaunchWizardPreviousProfiles::from_profile(previous_profile),
        )
    }

    pub fn open_start_work_with_previous_profiles(
        context: LaunchWizardContext,
        base_branch_name: String,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
        previous_profiles: LaunchWizardPreviousProfiles,
    ) -> Self {
        let mut state = Self::new_with(
            context,
            agent_options,
            quick_start_entries,
            previous_profiles,
            false,
        );
        state.wizard_mode = LaunchWizardMode::StartWork;
        state.step = LaunchWizardStep::LaunchTarget;
        state.launch_path = LaunchWizardLaunchPath::ManualSetup;
        state.selected = step_default_selection(state.step, &state);
        state.is_new_branch = true;
        state.base_branch_name = Some(base_branch_name);
        state.branch_name = state.context.normalized_branch_name.clone();
        state
    }

    pub fn open_start_work_with_previous_profile(
        context: LaunchWizardContext,
        base_branch_name: String,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
        previous_profile: Option<LaunchWizardPreviousProfile>,
    ) -> Self {
        Self::open_start_work_with_previous_profiles(
            context,
            base_branch_name,
            agent_options,
            quick_start_entries,
            LaunchWizardPreviousProfiles::from_profile(previous_profile),
        )
    }

    pub fn open_knowledge_launch_with_previous_profiles(
        context: LaunchWizardContext,
        base_branch_name: String,
        agent_options: Vec<AgentOption>,
        quick_start_entries: Vec<QuickStartEntry>,
        previous_profiles: LaunchWizardPreviousProfiles,
    ) -> Self {
        let mut state = Self::new_with(
            context,
            agent_options,
            quick_start_entries,
            previous_profiles,
            false,
        );
        state.wizard_mode = LaunchWizardMode::Knowledge;
        state.step = LaunchWizardStep::LaunchTarget;
        state.launch_path = LaunchWizardLaunchPath::ManualSetup;
        state.selected = step_default_selection(state.step, &state);
        state.is_new_branch = true;
        state.base_branch_name = Some(base_branch_name);
        state.branch_name = state.context.normalized_branch_name.clone();
        state
    }

    pub fn open_loading(context: LaunchWizardContext, agent_options: Vec<AgentOption>) -> Self {
        Self::new_with(
            context,
            agent_options,
            Vec::new(),
            LaunchWizardPreviousProfiles::default(),
            true,
        )
    }

    pub fn open(context: LaunchWizardContext, sessions_dir: &Path, cache_path: &Path) -> Self {
        let agent_options = load_agent_options(&gwt_agent::VersionCache::load(cache_path));
        let quick_start_entries = load_quick_start_entries(
            &context.quick_start_root,
            sessions_dir,
            &context.normalized_branch_name,
        );
        let previous_profiles = load_previous_launch_profiles(sessions_dir);
        Self::open_with_previous_profiles(
            context,
            agent_options,
            quick_start_entries,
            previous_profiles,
        )
    }

    pub fn apply_hydration(&mut self, hydration: LaunchWizardHydration) {
        let was_hydrating = self.is_hydrating;
        let preserve_runtime_selection = (!was_hydrating && self.runtime_context_resolved)
            || (self.runtime_resolution_pending
                && self.launch_path == LaunchWizardLaunchPath::QuickStart
                && self.selected_quick_start_index.is_some());
        let LaunchWizardHydration {
            selected_branch,
            normalized_branch_name,
            worktree_path,
            quick_start_root,
            docker_context,
            docker_service_status,
            agent_options,
            mut quick_start_entries,
            previous_profiles,
        } = hydration;
        if let Some(selected_branch) = selected_branch {
            self.context.selected_branch = selected_branch;
        }
        self.context.normalized_branch_name = normalized_branch_name;
        self.context.worktree_path = worktree_path;
        self.context.quick_start_root = quick_start_root;
        self.context.docker_context = docker_context;
        self.context.docker_service_status = docker_service_status;
        self.detected_agents = agent_options;
        Self::hydrate_live_window_ids(&self.context, &mut quick_start_entries);
        self.quick_start_entries = quick_start_entries;
        self.is_hydrating = false;
        self.runtime_context_resolved = true;
        self.runtime_resolution_pending = false;
        self.runtime_resolution_message = None;
        self.hydration_error = None;
        if was_hydrating {
            self.reset_default_launch_path();
        }
        self.branch_name = if self.is_new_branch {
            self.branch_name.clone()
        } else {
            self.context.normalized_branch_name.clone()
        };
        // SPEC-2014 FR-032..FR-035: hydration 経路でも初期化と同じ runtime resolver を使い、
        // open_loading -> hydration の間に repo-local Host/Docker 選好が失われないようにする。
        let refreshed_previous_profiles = previous_profiles.is_some();
        if let Some(previous_profiles) = previous_profiles {
            self.previous_profiles = previous_profiles;
        }
        if !preserve_runtime_selection {
            let (resolved_target, resolved_service, resolved_lifecycle) =
                resolve_initial_runtime_selection(
                    &self.context,
                    self.previous_profiles.repo_local(),
                );
            self.runtime_target = resolved_target;
            self.docker_service = resolved_service;
            self.docker_lifecycle_intent = resolved_lifecycle;
        }
        self.sync_selected_agent_options();
        // SPEC-2014 FR-054 / FR-056 (2026-05-15 Wizard Hydration Preserves User-Selected Agent):
        // hydration では preferred_agent_id で agent identity を上書きせず、
        // 現在選択 agent の per-agent draft / previous profile だけ refresh する。
        // preferred agent identity の適用は constructor (apply_preferred_agent_profile)
        // と set_agent_id 経由の明示的 agent 切替に限定する。
        if refreshed_previous_profiles && self.launch_path != LaunchWizardLaunchPath::QuickStart {
            self.save_current_agent_draft();
            self.restore_agent_draft_or_defaults();
        }
        self.sync_docker_lifecycle_default();
        self.selected = self
            .selected
            .min(self.current_options().len().saturating_sub(1));
    }

    pub fn mark_runtime_context_unresolved(&mut self) {
        self.runtime_context_resolved = false;
        self.runtime_resolution_pending = false;
        self.runtime_resolution_message = None;
        self.runtime_confirmed = false;
        self.settings_revisited = false;
        self.resolved_branch_name = None;
        self.context.worktree_path = None;
        self.context.docker_context = None;
        self.context.docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        self.runtime_target = gwt_agent::LaunchRuntimeTarget::Host;
        self.docker_service = None;
        self.docker_lifecycle_intent =
            default_docker_lifecycle_intent(self.context.docker_service_status);
        self.sync_docker_lifecycle_default();
    }

    pub fn mark_runtime_resolution_pending(&mut self, message: impl Into<String>) {
        self.runtime_context_resolved = false;
        self.runtime_resolution_pending = true;
        self.runtime_resolution_message = Some(message.into());
        self.runtime_confirmed = false;
        self.settings_revisited = false;
        self.error = None;
    }

    pub fn apply_runtime_context(&mut self, hydration: LaunchWizardHydration) {
        self.apply_hydration(hydration);
        if self.context.worktree_path.is_some() {
            self.is_new_branch = false;
            self.base_branch_name = None;
        }
        self.runtime_context_resolved = true;
        self.runtime_resolution_pending = false;
        self.runtime_resolution_message = None;
        // SPEC-2014 FR-127/FR-128: 解決完了直後は Runtime ステップ（Confirm 未確認）。
        self.runtime_confirmed = false;
        self.settings_revisited = false;
        self.resolved_branch_name = Some(self.branch_name.clone());
    }

    pub fn set_hydration_error(&mut self, error: String) {
        self.is_hydrating = false;
        self.runtime_resolution_pending = false;
        self.runtime_resolution_message = None;
        self.hydration_error = Some(error);
    }

    pub fn apply(&mut self, action: LaunchWizardAction) {
        self.error = None;
        if self.runtime_resolution_pending {
            match action {
                LaunchWizardAction::Cancel => {
                    self.completion = Some(LaunchWizardCompletion::Cancelled);
                }
                _ => return,
            }
            return;
        }

        match action {
            LaunchWizardAction::Cancel => {
                self.completion = Some(LaunchWizardCompletion::Cancelled);
            }
            LaunchWizardAction::Submit => {
                self.submit_panel();
            }
            LaunchWizardAction::GotoStep { phase } => {
                self.goto_phase(phase);
            }
            LaunchWizardAction::ApplyQuickStart { index, mode } => {
                self.apply_quick_start_action(index, mode);
            }
            LaunchWizardAction::UseStartMethod { method } => {
                self.use_start_method(method);
            }
            LaunchWizardAction::SetLaunchPath { path } => {
                self.set_launch_path_selection(path);
            }
            LaunchWizardAction::SelectQuickStart { index } => {
                self.select_quick_start(index);
            }
            LaunchWizardAction::SelectLiveSession { index } => {
                self.select_live_session(index);
            }
            LaunchWizardAction::FocusExistingSession { index } => {
                self.focus_existing_session(index);
            }
            LaunchWizardAction::SetBranchMode { create_new } => {
                self.set_branch_mode(create_new);
            }
            LaunchWizardAction::SetBranchType { prefix } => {
                self.set_branch_type(&prefix);
            }
            LaunchWizardAction::SetBranchName { value } => {
                self.branch_name = value;
            }
            LaunchWizardAction::SetLaunchTarget { target } => {
                self.set_launch_target(target);
            }
            LaunchWizardAction::SetAgent { agent_id } => {
                self.set_agent_id(&agent_id);
            }
            LaunchWizardAction::SetModel { model } => {
                self.set_model(&model);
            }
            LaunchWizardAction::SetReasoning { reasoning } => {
                self.set_reasoning(&reasoning);
            }
            LaunchWizardAction::SetRuntimeTarget { target } => {
                self.set_runtime_target(target);
            }
            LaunchWizardAction::SetWindowsShell { shell } => {
                self.windows_shell = shell;
            }
            LaunchWizardAction::SetDockerService { service } => {
                self.set_docker_service(&service);
            }
            LaunchWizardAction::SetDockerLifecycle { intent } => {
                self.set_docker_lifecycle(intent);
            }
            LaunchWizardAction::SetVersion { version } => {
                self.set_version(&version);
            }
            LaunchWizardAction::SetExecutionMode { mode } => {
                self.set_execution_mode(&mode);
            }
            LaunchWizardAction::SetSkipPermissions { enabled } => {
                self.skip_permissions = enabled;
            }
            LaunchWizardAction::SetLinkedIssue { issue_number } => {
                self.linked_issue_number = Some(issue_number);
            }
            LaunchWizardAction::ClearLinkedIssue => {
                self.linked_issue_number = None;
            }
            LaunchWizardAction::SetFastMode { enabled } => {
                self.codex_fast_mode = enabled && self.current_agent_supports_fast_mode();
            }
            LaunchWizardAction::SetCodexFastMode { enabled } => {
                self.codex_fast_mode = enabled && self.agent_is_codex();
            }
            LaunchWizardAction::Back => {
                if self.show_confirm() {
                    // SPEC-2014 FR-124: Confirm から Runtime ステップへ戻す。
                    self.runtime_confirmed = false;
                    return;
                }
                if self.show_runtime_confirmation() {
                    // SPEC-2014 FR-124/FR-125/FR-128: Runtime から Settings フォームへ
                    // 戻す。runtime 解決結果・選択を破棄せず（resolved 保持）Settings を
                    // 再表示する。branch 不変なら次の前進で再解決しない（SC-082）。
                    self.settings_revisited = true;
                    return;
                }
                if self.show_back_button() {
                    self.start_method_selected = false;
                    self.launch_path = LaunchWizardLaunchPath::ManualSetup;
                    self.step = LaunchWizardStep::LaunchTarget;
                    self.selected = step_default_selection(self.step, self);
                    self.completion = None;
                } else if let Some(prev) = prev_step(self.step, self) {
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
                if self.completion.is_none() && self.error.is_none() {
                    self.advance_after_current_step();
                }
            }
        }
    }

    pub(super) fn advance_after_current_step(&mut self) {
        if self.completion.is_some() {
            return;
        }

        if let Some(next) = next_step(self.step, self) {
            self.step = next;
            self.selected = step_default_selection(next, self);
            return;
        }

        self.finish_launch_request();
    }

    pub(super) fn apply_selection(&mut self) {
        match self.step {
            LaunchWizardStep::QuickStart => match self.selected_quick_start_action() {
                QuickStartAction::ReuseEntry { .. } | QuickStartAction::StartNewEntry { .. } => {
                    self.apply_quick_start_selection();
                    self.sync_docker_lifecycle_default();
                }
                QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => {}
            },
            LaunchWizardStep::FocusExistingSession => {
                let Some((index, _)) = self.running_live_sessions().nth(self.selected) else {
                    self.error = Some("No running session is available".to_string());
                    return;
                };
                self.focus_existing_session(index);
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
                    self.seed_branch_name_for_prefix(prefix);
                }
            }
            LaunchWizardStep::LaunchTarget => {
                self.set_launch_target(if self.selected == 0 {
                    LaunchTargetKind::Agent
                } else {
                    LaunchTargetKind::Shell
                });
            }
            LaunchWizardStep::AgentSelect => {
                if let Some(agent_id) = self
                    .detected_agents
                    .get(self.selected)
                    .map(|agent| agent.id.clone())
                {
                    self.set_agent_id(&agent_id);
                }
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
            LaunchWizardStep::WindowsShell => {
                if let Some(option) = WINDOWS_SHELL_OPTIONS.get(self.selected) {
                    self.windows_shell = *option;
                }
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
                let options = self.execution_mode_step_options();
                if let Some(option) = options.get(self.selected) {
                    self.mode = option.value.to_string();
                }
            }
            LaunchWizardStep::SkipPermissions => {
                self.skip_permissions = self.selected == 0;
            }
            LaunchWizardStep::CodexFastMode => {
                self.codex_fast_mode =
                    self.selected == 0 && self.current_agent_supports_fast_mode();
            }
            LaunchWizardStep::BranchNameInput => {}
        }
    }

    pub(super) fn submit_panel(&mut self) {
        match self.launch_path {
            LaunchWizardLaunchPath::QuickStart => {
                self.submit_quick_start_path();
                return;
            }
            LaunchWizardLaunchPath::FocusSession => {
                match self.selected_live_session_index {
                    Some(index) => self.focus_existing_session(index),
                    None => self.error = Some("No running session is available".to_string()),
                }
                return;
            }
            LaunchWizardLaunchPath::ManualSetup => {}
        }

        if self.is_new_branch {
            let trimmed = self.branch_name.trim();
            if trimmed.is_empty() {
                self.error = Some("Branch name is required".to_string());
                return;
            }
            self.branch_name = trimmed.to_string();
        }

        // SPEC-2014 FR-128: Settings を再訪（解決済みのまま）してから Submit した場合、
        // branch が変わっていれば再解決し、不変ならそのまま Runtime ステップへ戻る。
        if self.manual_setup_initialized && self.settings_revisited {
            self.advance_settings_to_runtime();
            return;
        }

        // SPEC-2014 FR-127: ConfigureAndStart の Setup で Runtime ステップ（解決済み・
        // 未確認）から Submit すると Confirm ステップへ進む。実 Launch は Confirm での
        // み発生する。即起動系（manual_setup_initialized=false）は経由しない（FR-129）。
        if self.manual_setup_initialized && self.runtime_context_resolved && !self.runtime_confirmed
        {
            self.runtime_confirmed = true;
            return;
        }

        if self.manual_setup_initialized {
            self.mode = "normal".to_string();
            self.resume_session_id = None;
        }
        self.finish_launch_request();
    }

    /// SPEC-2014 FR-128: Settings 再訪状態から Runtime ステップへ進む。branch が
    /// 解決時から変わっていれば無副作用の再解決をトリガし、不変なら resolved を保った
    /// まま Runtime を再表示する（SC-082）。
    fn advance_settings_to_runtime(&mut self) {
        self.settings_revisited = false;
        if self.branch_changed_since_resolution() {
            self.runtime_context_resolved = false;
            self.finish_launch_request();
        }
    }

    fn branch_changed_since_resolution(&self) -> bool {
        self.resolved_branch_name.as_deref() != Some(self.branch_name.as_str())
    }

    /// SPEC-2014 FR-128: progress rail クリックで指定フェーズへ移動する。
    /// 未到達・未解決のフェーズへのジャンプは無視する。
    fn goto_phase(&mut self, target: WizardPhase) {
        if self.is_hydrating || self.runtime_resolution_pending {
            return;
        }
        match target {
            WizardPhase::Path => {
                self.start_method_selected = false;
                self.settings_revisited = false;
                self.runtime_confirmed = false;
                self.completion = None;
            }
            WizardPhase::Settings => {
                if !self.manual_setup_initialized {
                    return;
                }
                self.settings_revisited = true;
                self.runtime_confirmed = false;
            }
            WizardPhase::Runtime => {
                if !self.manual_setup_initialized || !self.runtime_context_resolved {
                    return;
                }
                if self.settings_revisited {
                    self.advance_settings_to_runtime();
                } else {
                    self.runtime_confirmed = false;
                }
            }
            WizardPhase::Confirm => {
                if !self.manual_setup_initialized || !self.runtime_context_resolved {
                    return;
                }
                if self.settings_revisited && self.branch_changed_since_resolution() {
                    return;
                }
                self.settings_revisited = false;
                self.runtime_confirmed = true;
            }
        }
    }

    pub(super) fn current_phase(&self) -> WizardPhase {
        if self.show_confirm() {
            WizardPhase::Confirm
        } else if self.show_runtime_confirmation() {
            WizardPhase::Runtime
        } else if self.show_manual_setup() {
            WizardPhase::Settings
        } else {
            WizardPhase::Path
        }
    }

    fn finish_launch_request(&mut self) {
        match self.build_launch_request() {
            Ok(config) => {
                self.completion = Some(if self.runtime_context_resolved {
                    LaunchWizardCompletion::Launch(Box::new(config))
                } else {
                    LaunchWizardCompletion::ResolveRuntime(Box::new(config))
                });
            }
            Err(error) => {
                self.error = Some(error);
            }
        }
    }

    fn submit_quick_start_path(&mut self) {
        let Some(index) = self.selected_quick_start_index else {
            self.error = Some("Quick start entry is unavailable".to_string());
            return;
        };
        let mode = self
            .quick_start_entries
            .get(index)
            .map(|entry| {
                if entry.can_reuse() {
                    QuickStartLaunchMode::Resume
                } else {
                    QuickStartLaunchMode::StartNew
                }
            })
            .unwrap_or(QuickStartLaunchMode::StartNew);
        if self.prepare_quick_start_launch(index, mode, self.runtime_context_resolved) {
            self.finish_launch_request();
        }
    }

    fn use_start_method(&mut self, method: LaunchWizardStartMethodKind) {
        match method {
            LaunchWizardStartMethodKind::ConfigureAndStart => {
                if !self.manual_setup_initialized {
                    self.apply_latest_start_settings();
                    self.manual_setup_initialized = true;
                }
                self.launch_path = LaunchWizardLaunchPath::ManualSetup;
                self.start_method_selected = true;
                self.runtime_confirmed = false;
                self.settings_revisited = false;
                self.step = LaunchWizardStep::LaunchTarget;
                self.selected = step_default_selection(self.step, self);
                self.completion = None;
            }
            LaunchWizardStartMethodKind::StartWithLastSettings => {
                if !self.has_previous_start_settings() {
                    self.error = Some("No previous launch settings are available".to_string());
                    return;
                }
                self.apply_latest_start_settings();
                self.launch_path = LaunchWizardLaunchPath::ManualSetup;
                self.start_method_selected = true;
                self.finish_launch_request();
            }
            LaunchWizardStartMethodKind::ContinueLastSession => {
                self.continue_latest_session();
            }
            LaunchWizardStartMethodKind::OpenSessionPicker => {
                self.open_agent_session_picker();
            }
            LaunchWizardStartMethodKind::FocusRunningSession => {
                self.focus_latest_running_session();
            }
        }
    }

    fn apply_latest_start_settings(&mut self) {
        if let Some((index, _)) = self.latest_quick_start_entry() {
            let previous_completion = self.completion.take();
            let previous_error = self.error.take();
            let applied =
                self.prepare_quick_start_launch(index, QuickStartLaunchMode::StartNew, false);
            if !applied {
                self.completion = previous_completion;
                self.error = previous_error;
                return;
            }
            self.completion = previous_completion;
            self.error = previous_error;
        } else {
            self.mode = "normal".to_string();
            self.resume_session_id = None;
        }
    }

    pub(super) fn has_previous_start_settings(&self) -> bool {
        self.latest_quick_start_entry().is_some()
            || self.previous_profiles.preferred_agent_id().is_some()
            || self.previous_profiles.repo_local().is_some()
    }

    fn continue_latest_session(&mut self) {
        let Some((index, entry)) = self
            .latest_quick_start_entry()
            .map(|(index, entry)| (index, entry.clone()))
        else {
            self.error = Some("No saved session is available".to_string());
            return;
        };
        self.selected_quick_start_index = Some(index);
        self.launch_path = LaunchWizardLaunchPath::QuickStart;
        self.start_method_selected = true;
        self.launch_target = LaunchTargetKind::Agent;
        self.agent_id = entry.agent_id.clone();
        self.sync_selected_agent_options();
        self.apply_quick_start_runtime_selection(&entry);
        self.apply_saved_model(entry.model.as_deref());
        if let Some(reasoning) = entry.reasoning.clone() {
            self.reasoning = reasoning;
        }
        if let Some(version) = entry.version.clone() {
            self.version = version;
        }
        self.skip_permissions = entry.skip_permissions;
        self.codex_fast_mode = entry.codex_fast_mode && self.current_agent_supports_fast_mode();
        if let Some(resume_session_id) = entry.resume_session_id.clone() {
            self.mode = "resume".to_string();
            self.resume_session_id = Some(resume_session_id);
        } else {
            self.mode = "continue".to_string();
            self.resume_session_id = None;
        }
        self.finish_launch_request();
    }

    fn open_agent_session_picker(&mut self) {
        self.launch_path = LaunchWizardLaunchPath::QuickStart;
        self.start_method_selected = true;
        self.launch_target = LaunchTargetKind::Agent;
        self.sync_selected_agent_options();
        if !self.current_agent_supports_resume_picker() {
            self.error = Some("Session picker is unavailable for this agent".to_string());
            return;
        }
        self.mode = "resume".to_string();
        self.resume_session_id = None;
        self.finish_launch_request();
    }

    fn focus_latest_running_session(&mut self) {
        let Some((index, _)) = self.latest_running_session() else {
            self.error = Some("No running session is available".to_string());
            return;
        };
        self.start_method_selected = true;
        self.focus_existing_session(index);
    }

    fn apply_quick_start_action(&mut self, index: usize, mode: QuickStartLaunchMode) {
        self.launch_path = LaunchWizardLaunchPath::QuickStart;
        self.start_method_selected = true;
        self.selected_quick_start_index = Some(index);
        if self.prepare_quick_start_launch(index, mode, false) {
            self.finish_launch_request();
        }
    }

    fn prepare_quick_start_launch(
        &mut self,
        index: usize,
        mode: QuickStartLaunchMode,
        preserve_runtime_selection: bool,
    ) -> bool {
        let Some(entry) = self.quick_start_entries.get(index).cloned() else {
            self.error = Some("Quick start entry is unavailable".to_string());
            return false;
        };

        self.launch_target = LaunchTargetKind::Agent;
        self.agent_id = entry.agent_id.clone();
        self.sync_selected_agent_options();
        if !preserve_runtime_selection {
            self.apply_quick_start_runtime_selection(&entry);
        }
        self.apply_saved_model(entry.model.as_deref());
        if let Some(reasoning) = entry.reasoning {
            self.reasoning = reasoning;
        }
        if let Some(version) = entry.version {
            self.version = version;
        }
        self.skip_permissions = entry.skip_permissions;
        self.codex_fast_mode = entry.codex_fast_mode && self.current_agent_supports_fast_mode();
        match mode {
            QuickStartLaunchMode::Resume => {
                if let Some(window_id) = entry.live_window_id {
                    self.completion = Some(LaunchWizardCompletion::FocusWindow { window_id });
                    false
                } else if let Some(resume_session_id) = entry.resume_session_id {
                    self.mode = "resume".to_string();
                    self.resume_session_id = Some(resume_session_id);
                    true
                } else {
                    self.error = Some("No saved session is available".to_string());
                    false
                }
            }
            QuickStartLaunchMode::StartNew => {
                self.mode = "normal".to_string();
                self.resume_session_id = None;
                true
            }
        }
    }

    fn apply_quick_start_runtime_selection(&mut self, entry: &QuickStartEntry) {
        self.runtime_target = entry.runtime_target;
        self.docker_service = entry.docker_service.clone();
        self.docker_lifecycle_intent = entry.docker_lifecycle_intent;
        self.sync_docker_lifecycle_default();
    }

    /// Apply the locally-preferred agent identity (and matching per-agent
    /// draft) to the wizard. Constructor-only entry point for SPEC-2014
    /// FR-024 / FR-026 (Local User Agent Preferences). MUST NOT be called from
    /// `apply_hydration` or other mid-wizard refresh paths because it
    /// overwrites `self.agent_id`, which would discard the user's explicit
    /// Settings-step selection (SPEC-2014 FR-054).
    fn apply_preferred_agent_profile(&mut self) -> bool {
        if let Some(agent_id) = self
            .previous_profiles
            .preferred_agent_id()
            .map(str::to_string)
        {
            if self
                .detected_agents
                .iter()
                .any(|agent| agent.id == agent_id)
            {
                self.launch_target = LaunchTargetKind::Agent;
                self.agent_id = agent_id;
            }
        }
        self.restore_agent_draft_or_defaults()
    }

    fn apply_previous_agent_preferences(&mut self, profile: LaunchWizardPreviousProfile) {
        self.apply_saved_model(profile.model.as_deref());
        if let Some(reasoning) = profile.reasoning.as_deref() {
            if self
                .current_reasoning_options()
                .iter()
                .any(|option| option.stored_value == reasoning)
            {
                self.reasoning = reasoning.to_string();
            }
        }
        self.sync_reasoning_state();
        if let Some(version) = profile.version.as_deref() {
            if self
                .current_version_options()
                .iter()
                .any(|option| option.value == version)
            {
                self.version = version.to_string();
            }
        }
        self.mode = execution_mode_value_from_session_mode(profile.session_mode).to_string();
        self.resume_session_id = None;
        self.skip_permissions = profile.skip_permissions;
        self.codex_fast_mode = profile.codex_fast_mode && self.current_agent_supports_fast_mode();
    }

    fn focus_existing_session(&mut self, index: usize) {
        if let Some(entry) = self
            .context
            .live_sessions
            .get(index)
            .filter(|entry| entry.runtime_status == crate::WindowProcessStatus::Running)
        {
            self.launch_path = LaunchWizardLaunchPath::FocusSession;
            self.start_method_selected = true;
            self.selected_live_session_index = Some(index);
            self.completion = Some(LaunchWizardCompletion::FocusWindow {
                window_id: entry.window_id.clone(),
            });
        } else {
            self.error = Some("No running session is available".to_string());
        }
    }

    fn set_launch_path_selection(&mut self, path: LaunchWizardLaunchPath) {
        match path {
            LaunchWizardLaunchPath::QuickStart => {
                if self.quick_start_entries.is_empty() {
                    self.error = Some("Quick start entry is unavailable".to_string());
                    return;
                }
                self.launch_path = path;
                self.start_method_selected = true;
                self.selected_quick_start_index.get_or_insert(0);
            }
            LaunchWizardLaunchPath::ManualSetup => {
                self.launch_path = path;
                self.start_method_selected = true;
            }
            LaunchWizardLaunchPath::FocusSession => {
                let Some((index, _)) = self.latest_running_session() else {
                    self.error = Some("No running session is available".to_string());
                    return;
                };
                self.launch_path = path;
                self.start_method_selected = true;
                self.selected_live_session_index = Some(index);
            }
        }
    }

    fn select_quick_start(&mut self, index: usize) {
        if self.quick_start_entries.get(index).is_none() {
            self.error = Some("Quick start entry is unavailable".to_string());
            return;
        }
        self.launch_path = LaunchWizardLaunchPath::QuickStart;
        self.selected_quick_start_index = Some(index);
    }

    fn select_live_session(&mut self, index: usize) {
        if self
            .context
            .live_sessions
            .get(index)
            .filter(|entry| entry.runtime_status == crate::WindowProcessStatus::Running)
            .is_none()
        {
            self.error = Some("No running session is available".to_string());
            return;
        }
        self.launch_path = LaunchWizardLaunchPath::FocusSession;
        self.selected_live_session_index = Some(index);
    }

    pub(super) fn set_branch_mode(&mut self, create_new: bool) {
        self.is_new_branch = create_new;
        if create_new {
            self.base_branch_name = Some(self.context.normalized_branch_name.clone());
            if self.branch_name.is_empty()
                || self.branch_name == self.context.normalized_branch_name
            {
                self.branch_name.clear();
                self.seed_branch_name_for_prefix(BRANCH_TYPE_PREFIXES[0]);
            }
        } else {
            self.base_branch_name = None;
            self.branch_name = self.context.normalized_branch_name.clone();
        }
    }

    pub(super) fn set_branch_type(&mut self, prefix: &str) {
        if !BRANCH_TYPE_PREFIXES
            .iter()
            .any(|candidate| candidate == &prefix)
        {
            self.error = Some("Branch type is unavailable".to_string());
            return;
        }
        self.seed_branch_name_for_prefix(prefix);
    }

    /// Apply `prefix` to `branch_name`. When the current name has no
    /// user-entered suffix (empty or just a known prefix), pre-fill from
    /// `LinkedIssueKind` + `linked_issue_number` per SPEC-2014 FR-024/025.
    /// User-entered suffixes are preserved (NFR-008).
    fn seed_branch_name_for_prefix(&mut self, prefix: &str) {
        let trimmed = self.branch_name.trim();
        let user_suffix = BRANCH_TYPE_PREFIXES
            .iter()
            .find_map(|known| trimmed.strip_prefix(known))
            .unwrap_or(trimmed)
            .trim_matches('/');
        if user_suffix.is_empty() {
            self.branch_name = match self.context.linked_issue_branch_suffix() {
                Some(seed) => format!("{prefix}{seed}"),
                None => prefix.to_string(),
            };
        } else {
            self.branch_name = format!("{prefix}{user_suffix}");
        }
    }

    pub(super) fn set_launch_target(&mut self, target: LaunchTargetKind) {
        self.launch_path = LaunchWizardLaunchPath::ManualSetup;
        if self.launch_target_is_agent() && target == LaunchTargetKind::Shell {
            self.save_current_agent_draft();
        }
        self.launch_target = target;
        if self.launch_target_is_shell() {
            self.mode = "normal".to_string();
            self.resume_session_id = None;
            self.skip_permissions = false;
            self.codex_fast_mode = false;
        } else {
            self.restore_agent_draft_or_defaults();
        }
    }

    pub(super) fn set_agent_id(&mut self, agent_id: &str) {
        match self
            .detected_agents
            .iter()
            .position(|candidate| candidate.id == agent_id)
        {
            Some(index) => {
                self.save_current_agent_draft();
                self.agent_id = agent_id.to_string();
                if self.step == LaunchWizardStep::AgentSelect {
                    self.selected = index;
                }
                self.restore_agent_draft_or_defaults();
            }
            _ => {
                self.error = Some("Agent option is unavailable".to_string());
            }
        }
    }

    pub(super) fn set_model(&mut self, model: &str) {
        if current_model_options(self.effective_agent_id())
            .iter()
            .any(|candidate| candidate == &model)
        {
            self.model = model.to_string();
            self.sync_reasoning_state();
        } else if model.is_empty() && !self.agent_has_models() {
            self.model.clear();
        } else {
            self.error = Some("Model option is unavailable".to_string());
        }
    }

    pub(super) fn set_reasoning(&mut self, reasoning: &str) {
        if self
            .current_reasoning_options()
            .iter()
            .any(|option| option.stored_value == reasoning)
        {
            self.reasoning = reasoning.to_string();
        } else {
            self.error = Some("Reasoning option is unavailable".to_string());
        }
    }

    pub(super) fn set_runtime_target(&mut self, target: gwt_agent::LaunchRuntimeTarget) {
        self.runtime_target = target;
        if self.runtime_target == gwt_agent::LaunchRuntimeTarget::Host {
            self.docker_service = None;
        } else if self.docker_service.is_none() {
            self.docker_service = self.preferred_docker_service().map(str::to_string);
        }
        self.sync_docker_lifecycle_default();
    }

    pub(super) fn set_docker_service(&mut self, service: &str) {
        if self
            .docker_service_options()
            .iter()
            .any(|candidate| candidate == service)
        {
            self.docker_service = Some(service.to_string());
            self.sync_docker_lifecycle_default();
        } else {
            self.error = Some("Docker service is unavailable".to_string());
        }
    }

    pub(super) fn set_docker_lifecycle(&mut self, intent: gwt_agent::DockerLifecycleIntent) {
        if self
            .docker_lifecycle_options()
            .iter()
            .any(|option| option.intent == intent)
        {
            self.docker_lifecycle_intent = intent;
        } else {
            self.error = Some("Docker lifecycle option is unavailable".to_string());
        }
    }

    pub(super) fn set_version(&mut self, version: &str) {
        if self
            .current_version_options()
            .iter()
            .any(|option| option.value == version)
        {
            self.version = version.to_string();
        } else {
            self.error = Some("Version option is unavailable".to_string());
        }
    }

    pub(super) fn set_execution_mode(&mut self, mode: &str) {
        if EXECUTION_MODE_OPTIONS
            .iter()
            .any(|option| option.value == mode)
        {
            self.mode = mode.to_string();
            if self.mode != "resume" {
                self.resume_session_id = None;
            }
        } else {
            self.error = Some("Execution mode is unavailable".to_string());
        }
    }

    fn reset_default_launch_path(&mut self) {
        self.launch_path = default_launch_path(&self.context, &self.quick_start_entries);
        if !self.quick_start_entries.is_empty() {
            self.selected_quick_start_index.get_or_insert(0);
        }
        if !self.context.live_sessions.is_empty() {
            self.selected_live_session_index.get_or_insert(0);
        }
    }

    pub(super) fn selected_branch_type_prefix(&self) -> Option<&'static str> {
        BRANCH_TYPE_PREFIXES
            .iter()
            .find(|prefix| self.branch_name.starts_with(**prefix))
            .copied()
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

        if !self.current_agent_supports_fast_mode() {
            self.codex_fast_mode = false;
        }
        self.sync_reasoning_state();
    }

    fn apply_saved_model(&mut self, model: Option<&str>) {
        let Some(model) = model else {
            return;
        };
        if current_model_options(self.effective_agent_id())
            .iter()
            .any(|candidate| candidate == &model)
        {
            self.model = model.to_string();
        }
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

    pub(super) fn reasoning_level_for_launch(&self) -> Option<&str> {
        match self.effective_agent_id() {
            "codex" if !self.reasoning.is_empty() => Some(self.reasoning.as_str()),
            "claude"
                if !self.reasoning.is_empty()
                    && is_claude_effort_capable_model(self.model.as_str()) =>
            {
                Some(self.reasoning.as_str())
            }
            _ => None,
        }
    }

    pub(super) fn launch_target_is_agent(&self) -> bool {
        self.launch_target == LaunchTargetKind::Agent
    }

    pub(super) fn launch_target_is_shell(&self) -> bool {
        self.launch_target == LaunchTargetKind::Shell
    }

    pub(super) fn selected_agent(&self) -> Option<&AgentOption> {
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

    pub(super) fn effective_agent_id(&self) -> &str {
        self.selected_agent()
            .map(|agent| agent.id.as_str())
            .unwrap_or(self.agent_id.as_str())
    }

    pub(super) fn agent_is_codex(&self) -> bool {
        self.launch_target_is_agent() && self.effective_agent_id() == "codex"
    }

    pub(super) fn current_agent_supports_fast_mode(&self) -> bool {
        self.launch_target_is_agent()
            && agent_id_from_key(self.effective_agent_id()).supports_fast_mode()
    }

    pub(super) fn fast_mode_enabled_for_current_agent(&self) -> bool {
        self.codex_fast_mode && self.current_agent_supports_fast_mode()
    }

    /// SPEC-2014 2026-05-18 amendment FR-D: filtered Execution Mode option
    /// list seen by both the wizard-step path (`current_options`) and the
    /// default-selection helper. Mirrors the LaunchWizardView's
    /// `execution_mode_options`.
    pub(super) fn execution_mode_step_options(&self) -> Vec<LaunchWizardOptionView> {
        execution_mode_options_view(self.current_agent_supports_resume_picker())
    }

    /// SPEC-2014 2026-05-18 amendment FR-C / FR-D:
    /// Whether the current Launch target agent supports an interactive resume
    /// picker. Used by Execution Mode option filtering and `Resume → Continue`
    /// downgrade in [`Self::normalize_execution_mode`].
    pub(super) fn current_agent_supports_resume_picker(&self) -> bool {
        if !self.launch_target_is_agent() {
            return false;
        }
        if let Some(custom) = self
            .selected_agent()
            .and_then(|agent| agent.custom_agent.as_ref())
        {
            return custom.supports_resume_picker;
        }
        agent_id_from_key(self.effective_agent_id()).supports_resume_picker()
    }

    pub(super) fn agent_has_models(&self) -> bool {
        self.launch_target_is_agent()
            && matches!(self.effective_agent_id(), "claude" | "codex" | "gemini")
    }

    pub(super) fn agent_uses_reasoning_step(&self) -> bool {
        if !self.launch_target_is_agent() {
            return false;
        }
        if self.agent_is_codex() {
            return true;
        }
        self.effective_agent_id() == "claude" && is_claude_effort_capable_model(self.model.as_str())
    }

    pub(super) fn has_docker_workflow(&self) -> bool {
        self.context.docker_context.is_some()
    }

    pub(super) fn show_windows_shell_selection(&self) -> bool {
        cfg!(windows) && self.windows_shell_for_launch().is_some()
    }

    pub(super) fn windows_shell_for_launch(&self) -> Option<gwt_agent::WindowsShellKind> {
        (cfg!(windows) && self.runtime_target == gwt_agent::LaunchRuntimeTarget::Host)
            .then_some(self.windows_shell)
    }

    pub(super) fn docker_service_prompt_required(&self) -> bool {
        self.context
            .docker_context
            .as_ref()
            .is_some_and(|ctx| ctx.services.len() > 1)
    }

    pub(super) fn preferred_docker_service(&self) -> Option<&str> {
        self.docker_service.as_deref().or_else(|| {
            self.context
                .docker_context
                .as_ref()
                .and_then(|ctx| ctx.suggested_service.as_deref())
        })
    }

    pub(super) fn docker_service_options(&self) -> Vec<String> {
        self.context
            .docker_context
            .as_ref()
            .map(|ctx| ctx.services.clone())
            .unwrap_or_default()
    }

    pub(super) fn docker_lifecycle_options(&self) -> &'static [DockerLifecycleOption] {
        match self.context.docker_service_status {
            gwt_docker::ComposeServiceStatus::Unknown => &[DockerLifecycleOption {
                label: "Connect or start then launch",
                description: "Resolve the Docker service state at launch time",
                intent: gwt_agent::DockerLifecycleIntent::Start,
            }],
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

    pub(super) fn current_version_options(&self) -> Vec<gwt_agent::VersionOption> {
        self.selected_agent()
            .map(|agent| self.current_version_options_for(agent))
            .unwrap_or_default()
    }

    fn current_agent_draft_key(&self) -> Option<String> {
        if !self.launch_target_is_agent() {
            return None;
        }
        if !self.agent_id.is_empty() {
            return Some(self.agent_id.clone());
        }
        self.detected_agents.first().map(|agent| agent.id.clone())
    }

    fn save_current_agent_draft(&mut self) {
        let Some(agent_id) = self.current_agent_draft_key() else {
            return;
        };
        self.agent_drafts.insert(
            agent_id,
            AgentLaunchDraft {
                model: self.model.clone(),
                reasoning: self.reasoning.clone(),
                version: self.version.clone(),
                mode: self.mode.clone(),
                resume_session_id: self.resume_session_id.clone(),
                skip_permissions: self.skip_permissions,
                codex_fast_mode: self.fast_mode_enabled_for_current_agent(),
            },
        );
    }

    fn restore_agent_draft_or_defaults(&mut self) -> bool {
        let draft = self.agent_drafts.get(&self.agent_id).cloned();
        let restored = if let Some(draft) = draft {
            self.apply_agent_draft(draft);
            true
        } else if let Some(profile) = self.previous_profiles.profile_for(&self.agent_id).cloned() {
            self.apply_previous_agent_preferences(profile);
            true
        } else {
            self.reset_agent_draft_defaults();
            false
        };
        self.sync_selected_agent_options();
        self.normalize_execution_mode();
        restored
    }

    fn apply_agent_draft(&mut self, draft: AgentLaunchDraft) {
        self.model = draft.model;
        self.reasoning = draft.reasoning;
        self.version = draft.version;
        self.mode = draft.mode;
        self.resume_session_id = draft.resume_session_id;
        self.skip_permissions = draft.skip_permissions;
        self.codex_fast_mode = draft.codex_fast_mode && self.current_agent_supports_fast_mode();
    }

    fn reset_agent_draft_defaults(&mut self) {
        self.model.clear();
        self.reasoning.clear();
        self.version.clear();
        self.mode = "normal".to_string();
        self.resume_session_id = None;
        self.skip_permissions = false;
        self.codex_fast_mode = false;
    }

    pub(super) fn normalize_execution_mode(&mut self) {
        // Unknown mode strings always fall back to Normal.
        if !EXECUTION_MODE_OPTIONS
            .iter()
            .any(|option| option.value == self.mode)
        {
            self.mode = "normal".to_string();
            self.resume_session_id = None;
            return;
        }
        // SPEC-2014 2026-05-18 amendment FR-E:
        // Downgrade Resume → Continue when the current agent does not support
        // an interactive picker (e.g. Gemini / OpenCode / OpenClaw / Hermes /
        // Copilot / custom agents without opt-in capability).
        if self.mode == "resume" && !self.current_agent_supports_resume_picker() {
            self.mode = "continue".to_string();
            self.resume_session_id = None;
            return;
        }
        if self.mode != "resume" {
            self.resume_session_id = None;
        }
    }

    pub(super) fn current_reasoning_options(&self) -> Vec<ReasoningDisplayOption> {
        if self.agent_is_codex() {
            CODEX_REASONING_OPTIONS.to_vec()
        } else if self.effective_agent_id() == "claude"
            && is_claude_opus_tier_model(self.model.as_str())
        {
            let mut options = CLAUDE_OPUS_REASONING_OPTIONS.to_vec();
            // `ultracode` is opt-in and only usable on Opus-tier models with
            // Claude Code >= 2.1.154 and workflows enabled. It is the last
            // non-default row, so removing it keeps every other index stable.
            if !self.current_claude_ultracode_supported() {
                options.retain(|option| option.stored_value != "ultracode");
            }
            options
        } else if self.effective_agent_id() == "claude" && self.model == "sonnet" {
            CLAUDE_SONNET_REASONING_OPTIONS.to_vec()
        } else {
            Vec::new()
        }
    }

    fn current_claude_ultracode_supported(&self) -> bool {
        if !self.context.claude_workflows_enabled {
            return false;
        }
        match self.version.as_str() {
            "latest" => true,
            "installed" | "" => self.context.ultracode_supported,
            version => gwt_agent::supports_ultracode(version, true),
        }
    }

    pub(super) fn selected_quick_start_action(&self) -> QuickStartAction {
        self.quick_start_actions()
            .get(self.selected)
            .copied()
            .unwrap_or(QuickStartAction::ChooseDifferent)
    }

    pub(super) fn selected_quick_start_entry(&self) -> Option<&QuickStartEntry> {
        match self.selected_quick_start_action() {
            QuickStartAction::ReuseEntry { index } | QuickStartAction::StartNewEntry { index } => {
                self.quick_start_entries.get(index)
            }
            QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => None,
        }
    }

    pub(super) fn latest_quick_start_entry(&self) -> Option<(usize, &QuickStartEntry)> {
        self.quick_start_entries.iter().enumerate().next()
    }

    pub(super) fn running_live_sessions(&self) -> impl Iterator<Item = (usize, &LiveSessionEntry)> {
        self.context
            .live_sessions
            .iter()
            .enumerate()
            .filter(|(_, session)| session.runtime_status == crate::WindowProcessStatus::Running)
    }

    pub(super) fn latest_running_session(&self) -> Option<(usize, &LiveSessionEntry)> {
        self.running_live_sessions().next()
    }

    pub(super) fn quick_start_actions(&self) -> Vec<QuickStartAction> {
        let mut actions = Vec::new();
        for (index, entry) in self.quick_start_entries.iter().enumerate() {
            if entry.can_reuse() {
                actions.push(QuickStartAction::ReuseEntry { index });
            }
            actions.push(QuickStartAction::StartNewEntry { index });
        }
        if self.latest_running_session().is_some() {
            actions.push(QuickStartAction::FocusExistingSession);
        }
        actions.push(QuickStartAction::ChooseDifferent);
        actions
    }

    fn apply_quick_start_selection(&mut self) {
        let selected_action = self.selected_quick_start_action();
        let Some(entry) = self.selected_quick_start_entry().cloned() else {
            return;
        };
        let selected_index = match selected_action {
            QuickStartAction::ReuseEntry { index } | QuickStartAction::StartNewEntry { index } => {
                index
            }
            QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => return,
        };
        self.launch_path = LaunchWizardLaunchPath::QuickStart;
        self.selected_quick_start_index = Some(selected_index);

        self.launch_target = LaunchTargetKind::Agent;
        self.agent_id = entry.agent_id.clone();
        if let Some(index) = self
            .detected_agents
            .iter()
            .position(|agent| agent.id == entry.agent_id)
        {
            self.selected = index;
        }
        self.sync_selected_agent_options();

        self.apply_quick_start_runtime_selection(&entry);
        self.apply_saved_model(entry.model.as_deref());
        if let Some(reasoning) = entry.reasoning {
            self.reasoning = reasoning;
        }
        if let Some(version) = entry.version {
            self.version = version;
        }
        self.skip_permissions = entry.skip_permissions;
        self.codex_fast_mode = entry.codex_fast_mode && self.current_agent_supports_fast_mode();

        match selected_action {
            QuickStartAction::ReuseEntry { .. } => {
                if let Some(window_id) = entry.live_window_id {
                    self.completion = Some(LaunchWizardCompletion::FocusWindow { window_id });
                } else if let Some(resume_session_id) = entry.resume_session_id {
                    self.mode = "resume".to_string();
                    self.resume_session_id = Some(resume_session_id);
                    self.finish_launch_request();
                } else {
                    self.error = Some("No saved session is available".to_string());
                }
            }
            QuickStartAction::StartNewEntry { .. } => {
                self.mode = "normal".to_string();
                self.resume_session_id = None;
                self.finish_launch_request();
            }
            QuickStartAction::FocusExistingSession | QuickStartAction::ChooseDifferent => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};

    use super::super::test_support::*;
    use super::*;

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
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        assert_eq!(state.step, LaunchWizardStep::QuickStart);
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

    fn create_new_with_prefix(state: &mut LaunchWizardState, prefix: &str) {
        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchType {
            prefix: prefix.to_string(),
        });
    }

    #[test]
    fn branch_seed_uses_issue_kind_when_create_new_then_feature_prefix() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 42),
            sample_agent_options(),
            Vec::new(),
        );
        create_new_with_prefix(&mut state, "feature/");
        assert_eq!(state.branch_name, "feature/issue-42");
    }

    #[test]
    fn branch_seed_uses_spec_kind_when_create_new_then_feature_prefix() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Spec, 2014),
            sample_agent_options(),
            Vec::new(),
        );
        create_new_with_prefix(&mut state, "feature/");
        assert_eq!(state.branch_name, "feature/spec-2014");
    }

    #[test]
    fn branch_seed_uses_issue_kind_when_alternative_prefix_selected() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 10),
            sample_agent_options(),
            Vec::new(),
        );
        create_new_with_prefix(&mut state, "bugfix/");
        assert_eq!(state.branch_name, "bugfix/issue-10");
    }

    #[test]
    fn branch_seed_omits_when_no_linked_issue_kind() {
        let mut state = LaunchWizardState::open_with(
            context(branch("develop"), "develop"),
            sample_agent_options(),
            Vec::new(),
        );
        create_new_with_prefix(&mut state, "feature/");
        assert_eq!(state.branch_name, "feature/");
    }

    #[test]
    fn branch_seed_respects_user_edit_when_prefix_changes() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 42),
            sample_agent_options(),
            Vec::new(),
        );
        create_new_with_prefix(&mut state, "feature/");
        state.apply(LaunchWizardAction::SetBranchName {
            value: "feature/custom-name".to_string(),
        });
        state.apply(LaunchWizardAction::SetBranchType {
            prefix: "bugfix/".to_string(),
        });
        assert_eq!(state.branch_name, "bugfix/custom-name");
    }

    #[test]
    fn branch_seed_omits_title_slug_for_spec_proposal_a() {
        let ctx =
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Spec, 2014);
        let suffix = ctx
            .linked_issue_branch_suffix()
            .expect("linked issue branch suffix");
        assert_eq!(suffix, "spec-2014");
    }

    #[test]
    fn branch_seed_create_new_then_default_prefix_seeds_branch_name() {
        let mut state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 7),
            sample_agent_options(),
            Vec::new(),
        );
        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        assert_eq!(state.branch_name, "feature/issue-7");
    }

    // SPEC-2014 2026-05-18 amendment FR-E / SC-D:
    // Switching to a picker-unsupported agent while Resume is selected must
    // downgrade to Continue and clear any stale resume_session_id.
    #[test]
    fn normalize_execution_mode_downgrades_resume_when_switching_to_gemini() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        state.agent_id = "claude".to_string();
        state.mode = "resume".to_string();
        state.resume_session_id = Some("stale-id".to_string());

        // Sanity: Claude keeps Resume.
        state.normalize_execution_mode();
        assert_eq!(state.mode, "resume");

        // Switch to Gemini → Resume downgrades to Continue.
        state.agent_id = "gemini".to_string();
        state.normalize_execution_mode();
        assert_eq!(state.mode, "continue");
        assert!(state.resume_session_id.is_none());
    }

    #[test]
    fn open_with_previous_profile_restores_agent_preferences_without_reusing_branch() {
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "gwt".to_string()],
            suggested_service: Some("api".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let state = LaunchWizardState::open_with_previous_profile(
            ctx,
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Continue,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.branch_name, "feature/current");
        assert_eq!(view.selected_agent_id, "codex");
        assert_eq!(view.selected_model, "gpt-5.5");
        assert_eq!(view.selected_reasoning, "high");
        assert_eq!(view.selected_version, "0.110.0");
        assert_eq!(view.selected_execution_mode, "continue");
        assert_eq!(view.selected_runtime_target, "docker");
        // SPEC-2014 FR-034: saved docker_service が現 context にあれば saved を採用する。
        assert_eq!(view.selected_docker_service.as_deref(), Some("gwt"));
        assert_eq!(view.selected_docker_lifecycle, "restart");
        assert!(view.skip_permissions);
        assert!(view.codex_fast_mode);

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.branch.as_deref(), Some("feature/current"));
        assert_eq!(config.session_mode, gwt_agent::SessionMode::Continue);
        assert!(config.resume_session_id.is_none());
        assert_eq!(config.linked_issue_number, None);
    }

    #[test]
    fn apply_hydration_preserves_repo_local_host_preference_when_docker_context_appears() {
        // CodeRabbit PR #2661 B2: open_loading -> hydration の途中で apply_hydration が
        // raw Docker context のみで runtime_target を上書きしないこと。
        let initial_ctx = context(branch("feature/current"), "feature/current");
        let mut state = LaunchWizardState::open_with_previous_profile(
            initial_ctx,
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: None,
                reasoning: None,
                version: None,
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: false,
                codex_fast_mode: false,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
                windows_shell: None,
            }),
        );
        assert_eq!(state.view().selected_runtime_target, "host");
        state.apply_hydration(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: None,
            quick_start_root: PathBuf::from("/tmp/quick_start_root"),
            docker_context: Some(DockerWizardContext {
                services: vec!["api".to_string()],
                suggested_service: Some("api".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: None,
        });
        let view = state.view();
        assert_eq!(view.selected_runtime_target, "host");
        assert!(view.selected_docker_service.is_none());
    }

    #[test]
    fn previous_profile_docker_service_falls_back_to_first_service_when_no_suggestion() {
        // CodeRabbit PR #2661 B3: saved docker_service が stale で context に
        // suggested_service が無い場合、services の最初の要素を採用する。
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["only".to_string()],
            suggested_service: None,
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let state = LaunchWizardState::open_with_previous_profile(
            ctx,
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: None,
                reasoning: None,
                version: None,
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: false,
                codex_fast_mode: false,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("missing".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("only"));
    }

    #[test]
    fn previous_profile_runtime_target_restores_host_with_docker_context_available() {
        // SPEC-2014 SC-018: saved=Host のとき、Docker context が検出されていても Host を初期値にする。
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "gwt".to_string()],
            suggested_service: Some("api".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let state = LaunchWizardState::open_with_previous_profile(
            ctx,
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: false,
                codex_fast_mode: false,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.selected_runtime_target, "host");
        assert!(!view.show_docker_service);
        assert!(!view.show_docker_lifecycle);
    }

    #[test]
    fn previous_profile_docker_service_and_lifecycle_restore_when_service_present_in_current_context(
    ) {
        // SPEC-2014 SC-019: saved=Docker + saved docker_service が現在 context にあれば、
        // runtime_target / docker_service / docker_lifecycle_intent を session の値で復元する。
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("api".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let state = LaunchWizardState::open_with_previous_profile(
            ctx,
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: false,
                codex_fast_mode: false,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("worker".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("worker"));
        assert_eq!(view.selected_docker_lifecycle, "restart");
    }

    #[test]
    fn start_work_mode_skips_branch_steps_and_hides_branch_controls() {
        let state = LaunchWizardState::open_start_work_with_previous_profile(
            context(branch("origin/main"), "work/20260504-1234"),
            "origin/main".to_string(),
            sample_agent_options(),
            Vec::new(),
            None,
        );

        let view = state.view();

        assert_eq!(state.step, LaunchWizardStep::LaunchTarget);
        assert_eq!(view.title, "Start Work");
        assert_eq!(view.mode, LaunchWizardMode::StartWork);
        assert!(!view.show_branch_controls);
        assert_eq!(view.branch_name, "work/20260504-1234");
        assert!(
            !view
                .launch_summary
                .iter()
                .any(|item| item.label == "Branch"),
            "Start Work should not surface the generated work branch as primary UI"
        );
        assert!(state.is_new_branch);
        assert_eq!(state.base_branch_name.as_deref(), Some("origin/main"));
    }

    #[test]
    fn start_work_launch_config_materializes_reserved_work_branch() {
        let state = LaunchWizardState::open_start_work_with_previous_profile(
            context(branch("origin/develop"), "work/20260504-1234"),
            "origin/develop".to_string(),
            sample_agent_options(),
            Vec::new(),
            None,
        );

        let config = state.build_launch_config().expect("launch config");

        assert_eq!(config.branch.as_deref(), Some("work/20260504-1234"));
        assert_eq!(config.base_branch.as_deref(), Some("origin/develop"));
        assert!(
            config.working_dir.is_none(),
            "Start Work must defer worktree materialization until launch confirmation"
        );
    }

    #[test]
    fn runtime_confirmation_without_worktree_preserves_start_work_base_branch() {
        let mut state = LaunchWizardState::open_start_work_with_previous_profile(
            context(branch("origin/develop"), "work/20260504-1234"),
            "origin/develop".to_string(),
            sample_agent_options(),
            Vec::new(),
            None,
        );
        state.mark_runtime_context_unresolved();

        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion,
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));
        state.completion = None;
        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "work/20260504-1234".to_string(),
            worktree_path: None,
            quick_start_root: PathBuf::from("/tmp/repo"),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(Default::default()),
        });

        state.apply(LaunchWizardAction::Submit);

        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(config))
                if matches!(
                    config.as_ref(),
                    LaunchWizardLaunchRequest::Agent(config)
                        if config.branch.as_deref() == Some("work/20260504-1234")
                            && config.base_branch.as_deref() == Some("origin/develop")
                            && config.working_dir.is_none()
                )
        ));
    }

    #[test]
    fn knowledge_launch_mode_uses_issue_target_branch_and_hides_branch_controls() {
        let target_branch = knowledge_launch_target_branch_name(LinkedIssueKind::Issue, 7);
        let state = LaunchWizardState::open_knowledge_launch_with_previous_profiles(
            context_with_linked_issue(branch("develop"), &target_branch, LinkedIssueKind::Issue, 7),
            "develop".to_string(),
            sample_agent_options(),
            Vec::new(),
            LaunchWizardPreviousProfiles::default(),
        );

        let view = state.view();

        assert_eq!(state.step, LaunchWizardStep::LaunchTarget);
        assert_eq!(view.title, "Launch Agent");
        assert_eq!(view.mode, LaunchWizardMode::Knowledge);
        assert_eq!(view.branch_name, "work/issue-7");
        assert_eq!(view.branch_mode, "create_new");
        assert!(!view.show_branch_controls);
        assert!(state.is_new_branch);
        assert_eq!(state.base_branch_name.as_deref(), Some("develop"));

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.branch.as_deref(), Some("work/issue-7"));
        assert_eq!(config.base_branch.as_deref(), Some("develop"));
        assert!(config.working_dir.is_none());
        assert_eq!(config.linked_issue_number, Some(7));
    }

    #[test]
    fn knowledge_launch_mode_uses_spec_target_branch_and_hides_linked_issue_section() {
        let target_branch = knowledge_launch_target_branch_name(LinkedIssueKind::Spec, 2014);
        let state = LaunchWizardState::open_knowledge_launch_with_previous_profiles(
            context_with_linked_issue(
                branch("develop"),
                &target_branch,
                LinkedIssueKind::Spec,
                2014,
            ),
            "develop".to_string(),
            sample_agent_options(),
            Vec::new(),
            LaunchWizardPreviousProfiles::default(),
        );

        let view = state.view();

        assert_eq!(view.mode, LaunchWizardMode::Knowledge);
        assert_eq!(view.branch_name, "feature/spec-2014");
        assert_eq!(view.branch_mode, "create_new");
        assert!(!view.show_branch_controls);
        assert!(!view.show_linked_issue);

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.branch.as_deref(), Some("feature/spec-2014"));
        assert_eq!(config.base_branch.as_deref(), Some("develop"));
        assert!(config.working_dir.is_none());
        assert_eq!(config.linked_issue_number, Some(2014));
    }

    #[test]
    fn previous_profile_docker_service_falls_back_to_current_suggestion() {
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let state = LaunchWizardState::open_with_previous_profile(
            ctx,
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("missing".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("worker"));
    }

    #[test]
    fn previous_profile_docker_runtime_falls_back_to_host_without_context() {
        let state = LaunchWizardState::open_with_previous_profile(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
                windows_shell: None,
            }),
        );

        let view = state.view();
        assert_eq!(view.selected_runtime_target, "host");
        assert!(view.selected_docker_service.is_none());
        assert!(!view.show_docker_lifecycle);
    }

    #[test]
    fn previous_profile_keeps_saved_builtin_agent_without_host_detection() {
        let mut options = sample_agent_options();
        options
            .iter_mut()
            .find(|option| option.id == "codex")
            .expect("codex option")
            .available = false;
        let state = LaunchWizardState::open_with_previous_profile(
            context(branch("feature/current"), "feature/current"),
            options,
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
                windows_shell: None,
            }),
        );

        assert_eq!(state.view().selected_agent_id, "codex");
        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
    }

    #[test]
    fn previous_profile_uses_builtin_agent_even_when_none_are_host_detected() {
        let mut options = sample_agent_options();
        for option in &mut options {
            option.available = false;
        }
        let state = LaunchWizardState::open_with_previous_profile(
            context(branch("feature/current"), "feature/current"),
            options,
            Vec::new(),
            Some(LaunchWizardPreviousProfile {
                agent_id: "codex".to_string(),
                model: Some("gpt-5.5".to_string()),
                reasoning: Some("high".to_string()),
                version: Some("0.110.0".to_string()),
                session_mode: gwt_agent::SessionMode::Normal,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
                windows_shell: None,
            }),
        );

        assert_eq!(state.view().selected_agent_id, "codex");
        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
    }

    #[test]
    fn set_agent_keeps_launch_config_on_selected_agent_when_index_is_stale() {
        let mut options = sample_agent_options();
        options
            .iter_mut()
            .find(|option| option.id == "claude")
            .expect("claude option")
            .available = false;
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/current"), "feature/current"),
            options,
            Vec::new(),
        );
        state.step = LaunchWizardStep::AgentSelect;
        state.selected = 0;

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });

        assert_eq!(state.error, None);
        assert_eq!(state.agent_id, "codex");
        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
    }

    #[test]
    fn hydration_syncs_docker_lifecycle_when_previous_profile_is_not_applicable() {
        let mut state = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );
        state.apply_hydration(LaunchWizardHydration {
            selected_branch: Some(branch("origin/feature/gui")),
            normalized_branch_name: "feature/gui".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/repo-feature")),
            quick_start_root: PathBuf::from("/tmp/repo-feature"),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(LaunchWizardPreviousProfiles::from_profile(Some(
                LaunchWizardPreviousProfile {
                    agent_id: "missing-agent".to_string(),
                    model: None,
                    reasoning: None,
                    version: None,
                    session_mode: gwt_agent::SessionMode::Normal,
                    skip_permissions: false,
                    codex_fast_mode: false,
                    runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                    docker_service: None,
                    docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::CreateAndStart,
                    windows_shell: None,
                },
            ))),
        });

        assert_eq!(
            state.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Connect
        );
    }

    #[test]
    fn hydration_refresh_preserves_open_wizard_agent_settings_without_reapplying_preferences() {
        let mut codex = sample_session_record(
            "feature/old",
            Path::new("/tmp/old-repo"),
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 5, 10, 9, 0, 0).unwrap(),
            None,
        );
        codex.model = Some("gpt-5.4".to_string());
        codex.reasoning_level = Some("xhigh".to_string());
        codex.tool_version = Some("0.110.0".to_string());
        codex.session_mode = gwt_agent::SessionMode::Continue;
        codex.skip_permissions = true;
        codex.codex_fast_mode = true;

        let mut state = LaunchWizardState::open_with_previous_profiles(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
            previous_launch_profiles_from_sessions(&[codex]),
        );
        assert_eq!(state.view().selected_reasoning, "xhigh");

        state.apply(LaunchWizardAction::SetReasoning {
            reasoning: "medium".to_string(),
        });
        state.apply_hydration(LaunchWizardHydration {
            selected_branch: Some(branch("feature/current")),
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/current-repo")),
            quick_start_root: PathBuf::from("/tmp/current-repo"),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::Unknown,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: None,
        });

        let view = state.view();
        assert_eq!(view.selected_agent_id, "codex");
        assert_eq!(view.selected_model, "gpt-5.4");
        assert_eq!(view.selected_reasoning, "medium");
        assert_eq!(view.selected_version, "0.110.0");
        assert_eq!(view.selected_execution_mode, "continue");
        assert!(view.skip_permissions);
        assert!(view.codex_fast_mode);
    }

    #[test]
    fn apply_runtime_context_preserves_user_selected_agent_after_settings_step() {
        // SPEC-2014 FR-054 / FR-056 (2026-05-15 Wizard Hydration Preserves
        // User-Selected Agent): Settings step で user が agent を切り替えた後、
        // Runtime confirmation 経路 (apply_runtime_context) が previous_profiles を
        // refresh しても user 選択 agent_id を上書きしてはならない。
        let codex_session = sample_session_record(
            "feature/old",
            Path::new("/tmp/old-repo"),
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 5, 10, 9, 0, 0).unwrap(),
            None,
        );
        let previous_profiles =
            previous_launch_profiles_from_sessions(std::slice::from_ref(&codex_session));
        let mut state = LaunchWizardState::open_with_previous_profiles(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
            previous_profiles.clone(),
        );
        assert_eq!(state.view().selected_agent_id, "codex");

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        assert_eq!(state.view().selected_agent_id, "claude");

        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: Some(branch("feature/current")),
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/current-repo")),
            quick_start_root: PathBuf::from("/tmp/current-repo"),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::Unknown,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(previous_profiles),
        });

        let view = state.view();
        assert_eq!(view.selected_agent_id, "claude");
        let config = state
            .build_launch_config()
            .expect("launch config builds for user-selected agent");
        assert_eq!(config.agent_id, gwt_agent::AgentId::ClaudeCode);
    }

    #[test]
    fn apply_runtime_context_preserves_user_execution_mode_after_settings_step() {
        for mode in ["resume", "continue", "normal"] {
            let codex_session = sample_session_record(
                "feature/old",
                Path::new("/tmp/old-repo"),
                gwt_agent::AgentId::Codex,
                Utc.with_ymd_and_hms(2026, 5, 10, 9, 0, 0).unwrap(),
                None,
            );
            let previous_profiles =
                previous_launch_profiles_from_sessions(std::slice::from_ref(&codex_session));
            let mut state = LaunchWizardState::open_with_previous_profiles(
                context(branch("feature/current"), "feature/current"),
                sample_agent_options(),
                Vec::new(),
                previous_profiles.clone(),
            );

            state.apply(LaunchWizardAction::SetExecutionMode {
                mode: mode.to_string(),
            });
            assert_eq!(state.view().selected_execution_mode, mode);

            state.apply_runtime_context(LaunchWizardHydration {
                selected_branch: Some(branch("feature/current")),
                normalized_branch_name: "feature/current".to_string(),
                worktree_path: Some(PathBuf::from("/tmp/current-repo")),
                quick_start_root: PathBuf::from("/tmp/current-repo"),
                docker_context: None,
                docker_service_status: gwt_docker::ComposeServiceStatus::Unknown,
                agent_options: sample_agent_options(),
                quick_start_entries: Vec::new(),
                previous_profiles: Some(previous_profiles),
            });

            assert_eq!(state.view().selected_execution_mode, mode);
        }
    }

    #[test]
    fn open_session_picker_start_method_launches_agent_picker() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
        );

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::OpenSessionPicker,
        });

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.agent_id, gwt_agent::AgentId::ClaudeCode);
                    assert_eq!(config.session_mode, gwt_agent::SessionMode::Resume);
                    assert!(config.resume_session_id.is_none());
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected launch completion, got {other:?}"),
        }
    }

    #[test]
    fn continue_last_session_without_exact_resume_id_uses_agent_latest_session() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            vec![quick_start_entry(
                "session-1",
                "codex",
                None,
                None,
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
            )],
        );

        let continue_method = state
            .view()
            .start_methods
            .into_iter()
            .find(|method| method.kind == "continue_last_session")
            .expect("continue start method");
        assert!(continue_method.enabled);

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ContinueLastSession,
        });

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.agent_id, gwt_agent::AgentId::Codex);
                    assert_eq!(config.session_mode, gwt_agent::SessionMode::Continue);
                    assert!(config.resume_session_id.is_none());
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected launch completion, got {other:?}"),
        }
    }

    #[test]
    fn custom_agent_cache_refresh_preserves_user_selected_agent() {
        // SPEC-2014 FR-054 / FR-056 (2026-05-15 Wizard Hydration Preserves
        // User-Selected Agent): mid-wizard custom agent cache refresh (FR-018) でも
        // user 選択 agent_id は preferred_agent_id で上書きされてはならない。
        let codex_session = sample_session_record(
            "feature/old",
            Path::new("/tmp/old-repo"),
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 5, 10, 9, 0, 0).unwrap(),
            None,
        );
        let previous_profiles =
            previous_launch_profiles_from_sessions(std::slice::from_ref(&codex_session));
        let mut state = LaunchWizardState::open_with_previous_profiles(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
            previous_profiles.clone(),
        );
        assert_eq!(state.view().selected_agent_id, "codex");

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        assert_eq!(state.view().selected_agent_id, "claude");

        state.apply_hydration(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/current-repo")),
            quick_start_root: PathBuf::from("/tmp/current-repo"),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::Unknown,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(previous_profiles),
        });

        let view = state.view();
        assert_eq!(view.selected_agent_id, "claude");
        let config = state
            .build_launch_config()
            .expect("launch config builds for user-selected agent");
        assert_eq!(config.agent_id, gwt_agent::AgentId::ClaudeCode);
    }

    #[test]
    fn runtime_context_resolution_preserves_claude_fast_mode_draft() {
        let mut state = LaunchWizardState::open_start_work_with_previous_profile(
            context(branch("origin/develop"), "work/20260527-fast"),
            "origin/develop".to_string(),
            sample_agent_options(),
            Vec::new(),
            None,
        );
        state.mark_runtime_context_unresolved();

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        state.apply(LaunchWizardAction::SetFastMode { enabled: true });
        assert!(state.view().fast_mode);

        state.apply(LaunchWizardAction::Submit);
        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::ResolveRuntime(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.agent_id, gwt_agent::AgentId::ClaudeCode);
                    assert!(config.fast_mode);
                }
                other => panic!("expected agent runtime resolve request, got {other:?}"),
            },
            other => panic!("expected runtime resolve request, got {other:?}"),
        }

        state.completion = None;
        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "work/20260527-fast".to_string(),
            worktree_path: None,
            quick_start_root: PathBuf::from("/tmp/repo"),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(Default::default()),
        });

        assert!(state.view().fast_mode);
        // SPEC-2014 FR-127: ConfigureAndStart は Runtime→Confirm→Launch の3段。
        state.apply(LaunchWizardAction::Submit); // Runtime -> Confirm
        assert!(state.completion.is_none());
        assert!(state.view().show_confirm);
        assert!(state.view().fast_mode);
        state.apply(LaunchWizardAction::Submit); // Confirm -> Launch
        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.agent_id, gwt_agent::AgentId::ClaudeCode);
                    assert!(config.fast_mode);
                    assert!(config.args.windows(2).any(|pair| {
                        pair[0] == "--settings" && pair[1].ends_with("claude-settings-fast.json")
                    }));
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected launch request, got {other:?}"),
        }
    }

    #[test]
    fn unsupported_agent_hides_and_ignores_fast_mode() {
        let mut agent_options = sample_agent_options();
        agent_options.push(AgentOption {
            id: "aider".to_string(),
            name: "Aider".to_string(),
            available: true,
            installed_version: None,
            versions: Vec::new(),
            custom_agent: None,
        });
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            agent_options,
            Vec::new(),
        );

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "aider".to_string(),
        });
        state.apply(LaunchWizardAction::SetFastMode { enabled: true });

        let view = state.view();
        assert_eq!(view.selected_agent_id, "aider");
        assert!(!view.show_fast_mode);
        assert!(!view.fast_mode);
        assert!(!view.show_codex_fast_mode);
        assert!(!view.codex_fast_mode);

        let config = state.build_launch_config().expect("launch config");
        assert_eq!(config.agent_id, gwt_agent::AgentId::Custom("aider".into()));
        assert!(!config
            .args
            .windows(2)
            .any(|pair| pair[0] == "--settings" && pair[1].contains("fastMode")));
    }

    #[test]
    fn switching_agents_restores_each_agents_open_wizard_draft() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });
        state.apply(LaunchWizardAction::SetModel {
            model: "gpt-5.4".to_string(),
        });
        state.apply(LaunchWizardAction::SetReasoning {
            reasoning: "high".to_string(),
        });
        state.apply(LaunchWizardAction::SetVersion {
            version: "0.110.0".to_string(),
        });
        state.apply(LaunchWizardAction::SetExecutionMode {
            mode: "continue".to_string(),
        });
        state.apply(LaunchWizardAction::SetSkipPermissions { enabled: true });
        state.apply(LaunchWizardAction::SetCodexFastMode { enabled: true });

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "claude".to_string(),
        });
        state.apply(LaunchWizardAction::SetModel {
            model: "sonnet".to_string(),
        });
        state.apply(LaunchWizardAction::SetReasoning {
            reasoning: "low".to_string(),
        });
        state.apply(LaunchWizardAction::SetVersion {
            version: "installed".to_string(),
        });
        state.apply(LaunchWizardAction::SetExecutionMode {
            mode: "normal".to_string(),
        });
        state.apply(LaunchWizardAction::SetSkipPermissions { enabled: false });

        let claude_view = state.view();
        assert_eq!(claude_view.selected_agent_id, "claude");
        assert_eq!(claude_view.selected_model, "sonnet");
        assert_eq!(claude_view.selected_reasoning, "low");
        assert_eq!(claude_view.selected_version, "installed");
        assert_eq!(claude_view.selected_execution_mode, "normal");
        assert!(!claude_view.skip_permissions);
        assert!(!claude_view.show_codex_fast_mode);
        assert!(!claude_view.codex_fast_mode);

        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });

        let codex_view = state.view();
        assert_eq!(codex_view.selected_agent_id, "codex");
        assert_eq!(codex_view.selected_model, "gpt-5.4");
        assert_eq!(codex_view.selected_reasoning, "high");
        assert_eq!(codex_view.selected_version, "0.110.0");
        assert_eq!(codex_view.selected_execution_mode, "continue");
        assert!(codex_view.skip_permissions);
        assert!(codex_view.codex_fast_mode);
    }

    #[test]
    fn open_loading_marks_wizard_as_hydrating() {
        let state = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );

        let view = state.view();
        assert!(state.is_hydrating);
        assert!(view.is_hydrating);
        assert!(state.quick_start_entries.is_empty());
        assert!(!view.show_runtime_target);
        assert!(view.hydration_error.is_none());
    }

    #[test]
    fn apply_hydration_updates_docker_defaults_and_quick_start_entries() {
        let mut state = LaunchWizardState::open_loading(
            context(branch("feature/gui"), "feature/gui"),
            Vec::new(),
        );
        let worktree = PathBuf::from("/tmp/repo-feature");
        state.apply_hydration(LaunchWizardHydration {
            selected_branch: Some(branch("origin/feature/gui")),
            normalized_branch_name: "feature/gui".to_string(),
            worktree_path: Some(worktree.clone()),
            quick_start_root: worktree.clone(),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string(), "worker".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: vec![QuickStartEntry {
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
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
            previous_profiles: Some(LaunchWizardPreviousProfiles::default()),
        });

        let view = state.view();
        assert!(!state.is_hydrating);
        assert_eq!(
            state.context.worktree_path.as_deref(),
            Some(worktree.as_path())
        );
        assert_eq!(state.context.normalized_branch_name, "feature/gui");
        assert_eq!(state.runtime_target, gwt_agent::LaunchRuntimeTarget::Docker);
        assert_eq!(state.docker_service.as_deref(), Some("app"));
        assert_eq!(
            state.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Connect
        );
        assert_eq!(state.quick_start_entries.len(), 1);
        assert!(view.show_runtime_target);
        assert!(!view.is_hydrating);
        assert_eq!(view.selected_agent_id, "claude");
        assert_eq!(view.agent_options.len(), 2);
        assert_eq!(view.selected_runtime_target, "docker");
    }
}
