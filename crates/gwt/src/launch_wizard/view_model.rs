use super::*;

fn start_method_group(
    kind: LaunchWizardStartMethodKind,
    enabled: bool,
    recommended_method: LaunchWizardStartMethodKind,
) -> String {
    if !enabled {
        return "unavailable".to_string();
    }
    if kind == recommended_method {
        "recommended".to_string()
    } else {
        "available".to_string()
    }
}

impl LaunchWizardState {
    pub fn view(&self) -> LaunchWizardView {
        let show_start_methods = self.show_start_methods();
        let show_back_button = self.show_back_button();
        let show_manual_setup = self.show_manual_setup();
        let show_runtime_confirmation = self.show_runtime_confirmation();
        let show_fast_mode = show_manual_setup
            && self.launch_target_is_agent()
            && self.current_agent_supports_fast_mode();
        let fast_mode = self.fast_mode_enabled_for_current_agent();
        LaunchWizardView {
            title: if self.wizard_mode == LaunchWizardMode::StartWork {
                "Start Work".to_string()
            } else {
                "Launch Agent".to_string()
            },
            mode: self.wizard_mode,
            branch_name: self.branch_name.clone(),
            selected_branch_name: self.context.selected_branch.name.clone(),
            linked_issue_number: self.linked_issue_number,
            is_hydrating: self.is_hydrating,
            runtime_context_resolved: self.runtime_context_resolved,
            hydration_error: self.hydration_error.clone(),
            start_methods: self.start_methods_view(),
            quick_start_entries: self.quick_start_entries_view(),
            live_sessions: self.live_sessions_view(),
            selected_launch_path: self.launch_path.value().to_string(),
            selected_quick_start_index: self.selected_quick_start_index,
            selected_live_session_index: self.selected_live_session_index,
            branch_mode: if self.is_new_branch {
                "create_new".to_string()
            } else {
                "use_selected".to_string()
            },
            branch_type_options: branch_type_options_view(),
            selected_branch_type: self.selected_branch_type_prefix().map(str::to_string),
            launch_target_options: launch_target_options_view(),
            selected_launch_target: launch_target_value(self.launch_target).to_string(),
            agent_options: self.agent_options_view(),
            selected_agent_id: self.effective_agent_id().to_string(),
            model_options: self.model_options_view(),
            selected_model: self.model.clone(),
            reasoning_options: self.reasoning_options_view(),
            selected_reasoning: self.reasoning.clone(),
            runtime_target_options: runtime_target_options_view(),
            selected_runtime_target: runtime_target_value(self.runtime_target).to_string(),
            windows_shell_options: windows_shell_options_view(),
            selected_windows_shell: self
                .windows_shell_for_launch()
                .map(|shell| windows_shell_option_value(shell).to_string()),
            docker_service_options: self.docker_service_options_view(),
            selected_docker_service: self.docker_service.clone(),
            docker_lifecycle_options: self.docker_lifecycle_options_view(),
            selected_docker_lifecycle: docker_lifecycle_value(self.docker_lifecycle_intent)
                .to_string(),
            version_options: self.version_options_view(),
            selected_version: self.version.clone(),
            execution_mode_options: execution_mode_options_view(
                self.current_agent_supports_resume_picker(),
            ),
            selected_execution_mode: self.mode.clone(),
            skip_permissions: self.skip_permissions,
            show_agent_settings: show_manual_setup && self.launch_target_is_agent(),
            show_reasoning: show_manual_setup
                && self.launch_target_is_agent()
                && self.agent_uses_reasoning_step(),
            show_runtime_target: show_runtime_confirmation && self.has_docker_workflow(),
            show_windows_shell: self.runtime_context_resolved
                && show_manual_setup
                && self.show_windows_shell_selection(),
            show_docker_service: self.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker
                && show_runtime_confirmation
                && self.docker_service_prompt_required(),
            show_docker_lifecycle: self.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker
                && show_runtime_confirmation,
            show_version: show_manual_setup
                && self.launch_target_is_agent()
                && agent_has_npm_package(self.effective_agent_id()),
            show_execution_mode: false,
            show_skip_permissions: show_manual_setup && self.launch_target_is_agent(),
            show_fast_mode,
            show_codex_fast_mode: show_manual_setup
                && self.launch_target_is_agent()
                && self.agent_is_codex(),
            show_branch_controls: show_manual_setup && self.wizard_mode == LaunchWizardMode::Branch,
            show_start_methods,
            show_back_button,
            show_manual_setup,
            show_runtime_confirmation,
            show_confirm: self.show_confirm(),
            show_linked_issue: matches!(
                self.context.linked_issue_kind,
                Some(LinkedIssueKind::Issue)
            ) && self.linked_issue_number.is_some(),
            runtime_resolution_pending: self.runtime_resolution_pending,
            runtime_resolution_message: self.runtime_resolution_message.clone(),
            primary_action_label: self.primary_action_label(),
            primary_action_enabled: self.primary_action_enabled(),
            progress_steps: self.progress_steps_view(),
            fast_mode,
            codex_fast_mode: self.codex_fast_mode && self.agent_is_codex(),
            launch_summary: self.launch_summary_view(),
            phase: self.current_phase(),
            error: self.error.clone(),
        }
    }

    fn start_methods_view(&self) -> Vec<LaunchWizardStartMethodView> {
        let has_previous_settings = self.has_previous_start_settings();
        let latest_session = self.latest_quick_start_entry().map(|(_, entry)| entry);
        let latest_live = self.latest_running_session().map(|(_, session)| session);
        let latest_session_can_continue = latest_session
            .map(|entry| self.quick_start_entry_supports_session_continuation(entry))
            .unwrap_or(false);
        let recommended_method = if latest_live.is_some() {
            LaunchWizardStartMethodKind::FocusRunningSession
        } else if latest_session_can_continue {
            LaunchWizardStartMethodKind::ContinueLastSession
        } else if has_previous_settings {
            LaunchWizardStartMethodKind::StartWithLastSettings
        } else {
            LaunchWizardStartMethodKind::ConfigureAndStart
        };

        let mut methods = vec![
            LaunchWizardStartMethodView {
                kind: LaunchWizardStartMethodKind::ConfigureAndStart
                    .value()
                    .to_string(),
                label: "Configure and start".to_string(),
                badge: "Settings".to_string(),
                group: start_method_group(
                    LaunchWizardStartMethodKind::ConfigureAndStart,
                    true,
                    recommended_method,
                ),
                recommended: recommended_method == LaunchWizardStartMethodKind::ConfigureAndStart,
                summary: "Edit settings before launch".to_string(),
                detail: None,
                enabled: true,
                disabled_reason: None,
            },
            LaunchWizardStartMethodView {
                kind: LaunchWizardStartMethodKind::StartWithLastSettings
                    .value()
                    .to_string(),
                label: "Start with last settings".to_string(),
                badge: "New".to_string(),
                group: start_method_group(
                    LaunchWizardStartMethodKind::StartWithLastSettings,
                    has_previous_settings,
                    recommended_method,
                ),
                recommended: recommended_method
                    == LaunchWizardStartMethodKind::StartWithLastSettings,
                summary: if has_previous_settings {
                    "New session with saved settings"
                } else {
                    "Use saved launch settings"
                }
                .to_string(),
                detail: None,
                enabled: has_previous_settings,
                disabled_reason: (!has_previous_settings).then(|| "None saved yet".to_string()),
            },
            LaunchWizardStartMethodView {
                kind: LaunchWizardStartMethodKind::ContinueLastSession
                    .value()
                    .to_string(),
                label: "Continue last session".to_string(),
                badge: "Session".to_string(),
                group: start_method_group(
                    LaunchWizardStartMethodKind::ContinueLastSession,
                    latest_session_can_continue,
                    recommended_method,
                ),
                recommended: recommended_method == LaunchWizardStartMethodKind::ContinueLastSession,
                summary: latest_session
                    .map(|entry| {
                        if !self.quick_start_entry_supports_session_continuation(entry) {
                            "Resume recent session"
                        } else if entry.resume_session_id.is_some() {
                            "Resume conversation history"
                        } else {
                            "Continue latest agent session"
                        }
                    })
                    .unwrap_or("Resume recent session")
                    .to_string(),
                detail: latest_session
                    .and_then(|entry| entry.resume_session_id.as_deref())
                    .map(|resume_id| format!("Resume ID · {resume_id}")),
                enabled: latest_session_can_continue,
                disabled_reason: if latest_session.is_some() && !latest_session_can_continue {
                    Some("Not supported by agent".to_string())
                } else if latest_session.is_none() {
                    Some("No saved session".to_string())
                } else {
                    None
                },
            },
        ];
        if self.current_agent_supports_resume_picker() {
            methods.push(LaunchWizardStartMethodView {
                kind: LaunchWizardStartMethodKind::OpenSessionPicker
                    .value()
                    .to_string(),
                label: "Open session picker".to_string(),
                badge: "Picker".to_string(),
                group: start_method_group(
                    LaunchWizardStartMethodKind::OpenSessionPicker,
                    true,
                    recommended_method,
                ),
                recommended: recommended_method == LaunchWizardStartMethodKind::OpenSessionPicker,
                summary: "Choose a saved session".to_string(),
                detail: Some("Opens the agent's session picker".to_string()),
                enabled: true,
                disabled_reason: None,
            });
        }
        methods.push(LaunchWizardStartMethodView {
            kind: LaunchWizardStartMethodKind::FocusRunningSession
                .value()
                .to_string(),
            label: "Focus running session".to_string(),
            badge: "Running".to_string(),
            group: start_method_group(
                LaunchWizardStartMethodKind::FocusRunningSession,
                latest_live.is_some(),
                recommended_method,
            ),
            recommended: recommended_method == LaunchWizardStartMethodKind::FocusRunningSession,
            summary: latest_live
                .map(|session| session.name.clone())
                .unwrap_or_else(|| "Switch to running session".to_string()),
            detail: latest_live.and_then(|session| {
                session
                    .detail
                    .clone()
                    .or_else(|| Some(live_session_status_label(session)))
            }),
            enabled: latest_live.is_some(),
            disabled_reason: latest_live
                .is_none()
                .then(|| "No running session".to_string()),
        });
        methods
    }

    fn quick_start_entries_view(&self) -> Vec<LaunchWizardQuickStartView> {
        self.quick_start_entries
            .iter()
            .enumerate()
            .map(|(index, entry)| LaunchWizardQuickStartView {
                index,
                tool_label: entry.tool_label.clone(),
                summary: quick_start_summary(entry),
                resume_session_id: entry.resume_session_id.clone(),
                reuse_action_label: entry.reuse_action_label().map(str::to_string),
            })
            .collect()
    }

    fn live_sessions_view(&self) -> Vec<LaunchWizardLiveSessionView> {
        self.context
            .live_sessions
            .iter()
            .enumerate()
            .map(|(index, entry)| LaunchWizardLiveSessionView {
                index,
                name: entry.name.clone(),
                detail: entry.detail.clone(),
                active: entry.active,
                runtime_status: window_status_wire(entry.runtime_status).to_string(),
            })
            .collect()
    }

    fn agent_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.detected_agents
            .iter()
            .map(|agent| LaunchWizardOptionView {
                value: agent.id.clone(),
                label: agent.name.clone(),
                description: Some(agent_description(agent)),
                color: agent_option_color(&agent.id),
            })
            .collect()
    }

    fn model_options_view(&self) -> Vec<LaunchWizardOptionView> {
        model_display_options(self.effective_agent_id())
            .iter()
            .map(|option| LaunchWizardOptionView {
                value: option.label.to_string(),
                label: option.label.to_string(),
                description: Some(option.description.to_string()),
                color: None,
            })
            .collect()
    }

    fn reasoning_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.current_reasoning_options()
            .iter()
            .map(|option| LaunchWizardOptionView {
                value: option.stored_value.to_string(),
                label: option.label.to_string(),
                description: Some(option.description.to_string()),
                color: None,
            })
            .collect()
    }

    fn docker_service_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.docker_service_options()
            .into_iter()
            .map(|service| LaunchWizardOptionView {
                value: service.clone(),
                label: service,
                description: Some("Docker Compose service".to_string()),
                color: None,
            })
            .collect()
    }

    fn docker_lifecycle_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.docker_lifecycle_options()
            .iter()
            .map(|option| LaunchWizardOptionView {
                value: docker_lifecycle_value(option.intent).to_string(),
                label: option.label.to_string(),
                description: Some(option.description.to_string()),
                color: None,
            })
            .collect()
    }

    fn version_options_view(&self) -> Vec<LaunchWizardOptionView> {
        self.current_version_options()
            .into_iter()
            .map(|option| LaunchWizardOptionView {
                value: option.value,
                label: option.label,
                description: Some("Tool version".to_string()),
                color: None,
            })
            .collect()
    }

    fn launch_summary_view(&self) -> Vec<LaunchWizardSummaryView> {
        let mut summary = if self.wizard_mode == LaunchWizardMode::StartWork {
            vec![LaunchWizardSummaryView {
                label: "Workspace".to_string(),
                value: "Current project".to_string(),
            }]
        } else {
            vec![LaunchWizardSummaryView {
                label: "Branch".to_string(),
                value: self.branch_name.clone(),
            }]
        };
        summary.push(LaunchWizardSummaryView {
            label: "Target".to_string(),
            value: match self.launch_target {
                LaunchTargetKind::Agent => "Agent".to_string(),
                LaunchTargetKind::Shell => "Shell".to_string(),
            },
        });

        if self.launch_target_is_agent() {
            summary.push(LaunchWizardSummaryView {
                label: "Agent".to_string(),
                value: self
                    .selected_agent()
                    .map(|agent| agent.name.clone())
                    .unwrap_or_else(|| "Unavailable".to_string()),
            });
            if is_explicit_model_selection(&self.model) {
                summary.push(LaunchWizardSummaryView {
                    label: "Model".to_string(),
                    value: self.model.clone(),
                });
            }
            if let Some(reasoning) = self.reasoning_level_for_launch() {
                summary.push(LaunchWizardSummaryView {
                    label: if self.agent_is_codex() {
                        "Reasoning".to_string()
                    } else {
                        "Effort".to_string()
                    },
                    value: reasoning.to_string(),
                });
            }
            if !self.version.is_empty() {
                summary.push(LaunchWizardSummaryView {
                    label: "Version".to_string(),
                    value: self.version.clone(),
                });
            }
        }
        summary.push(LaunchWizardSummaryView {
            label: "Runtime".to_string(),
            value: if self.runtime_target == gwt_agent::LaunchRuntimeTarget::Docker {
                self.docker_service
                    .as_ref()
                    .map(|service| format!("docker:{service}"))
                    .unwrap_or_else(|| "docker".to_string())
            } else {
                "host".to_string()
            },
        });
        if let Some(windows_shell) = self.windows_shell_for_launch() {
            summary.push(LaunchWizardSummaryView {
                label: "Shell".to_string(),
                value: windows_shell_option_label(windows_shell).to_string(),
            });
        }
        if self.launch_target_is_agent() {
            summary.push(LaunchWizardSummaryView {
                label: "Permissions".to_string(),
                value: if self.skip_permissions {
                    "skip".to_string()
                } else {
                    "prompt".to_string()
                },
            });
        }
        if self.current_agent_supports_fast_mode() {
            summary.push(LaunchWizardSummaryView {
                label: "Fast mode".to_string(),
                value: if self.fast_mode_enabled_for_current_agent() {
                    "on".to_string()
                } else {
                    "off".to_string()
                },
            });
        }

        summary
    }

    pub(super) fn show_manual_setup(&self) -> bool {
        // SPEC-2014 FR-126: Settings フォームは Runtime / Confirm ステップでない時のみ。
        // settings_revisited（解決済みのまま Settings 再訪）でも true になる。
        self.launch_path == LaunchWizardLaunchPath::ManualSetup
            && !self.show_start_methods()
            && !self.show_runtime_confirmation()
            && !self.show_confirm()
    }

    pub(super) fn show_back_button(&self) -> bool {
        // SPEC-2014 FR-123: Back は Path 入口以外の全フェーズ（Settings/Runtime/
        // Confirm）で表示する。runtime 確認画面でも非表示にしない。
        self.start_method_selected
            && self.launch_path == LaunchWizardLaunchPath::ManualSetup
            && !self.is_hydrating
            && !self.runtime_resolution_pending
    }

    fn show_start_methods(&self) -> bool {
        !self.start_method_selected
            && !self.is_hydrating
            && !self.runtime_resolution_pending
            && !self.show_runtime_confirmation()
    }

    pub(super) fn show_runtime_confirmation(&self) -> bool {
        // SPEC-2014 FR-126: Runtime ステップ（解決済みだが Confirm 未確認）。
        // ConfigureAndStart の Setup 3ステップで Confirm へ進むと Runtime 編集 UI を
        // 隠す。QuickStart / 即起動系（manual_setup_initialized=false）は Confirm を
        // 使わず従来どおり確認即起動する（FR-129）。
        self.runtime_context_resolved
            && !self.settings_revisited
            && !(self.runtime_confirmed && self.manual_setup_initialized)
            && matches!(
                self.launch_path,
                LaunchWizardLaunchPath::QuickStart | LaunchWizardLaunchPath::ManualSetup
            )
    }

    /// SPEC-2014 FR-127: ManualSetup（ConfigureAndStart の Setup 3ステップ）の Confirm
    /// ステップ（読み取りサマリ + Launch）。Runtime ステップで Submit すると
    /// `runtime_confirmed` が立ちここに入る。即起動系は経由しない（FR-129）。
    pub(super) fn show_confirm(&self) -> bool {
        self.runtime_context_resolved
            && !self.settings_revisited
            && self.runtime_confirmed
            && self.manual_setup_initialized
            && self.launch_path == LaunchWizardLaunchPath::ManualSetup
    }

    fn primary_action_label(&self) -> String {
        if self.is_hydrating {
            return "Loading...".to_string();
        }
        if self.runtime_resolution_pending {
            return "Preparing...".to_string();
        }
        if self.show_start_methods() {
            return "Choose start method".to_string();
        }
        match self.launch_path {
            LaunchWizardLaunchPath::FocusSession => "Focus".to_string(),
            LaunchWizardLaunchPath::QuickStart | LaunchWizardLaunchPath::ManualSetup
                if !self.runtime_context_resolved =>
            {
                "Continue".to_string()
            }
            // SPEC-2014 FR-127: ConfigureAndStart の Setup で Runtime ステップ（解決済み
            // だが未確認）は Confirm へ進む「Continue」。実 Launch は Confirm でのみ。
            LaunchWizardLaunchPath::ManualSetup
                if self.manual_setup_initialized && !self.runtime_confirmed =>
            {
                "Continue".to_string()
            }
            LaunchWizardLaunchPath::QuickStart | LaunchWizardLaunchPath::ManualSetup
                if self.is_new_branch =>
            {
                "Create and launch".to_string()
            }
            LaunchWizardLaunchPath::QuickStart | LaunchWizardLaunchPath::ManualSetup => {
                "Launch".to_string()
            }
        }
    }

    fn primary_action_enabled(&self) -> bool {
        if self.is_hydrating || self.runtime_resolution_pending || self.show_start_methods() {
            return false;
        }
        match self.launch_path {
            LaunchWizardLaunchPath::QuickStart => self
                .selected_quick_start_index
                .is_some_and(|index| self.quick_start_entries.get(index).is_some()),
            LaunchWizardLaunchPath::FocusSession => self
                .selected_live_session_index
                .is_some_and(|index| self.context.live_sessions.get(index).is_some()),
            LaunchWizardLaunchPath::ManualSetup => true,
        }
    }

    fn progress_steps_view(&self) -> Vec<LaunchWizardProgressStepView> {
        let path_label = match self.launch_path {
            LaunchWizardLaunchPath::QuickStart => "Quick Start",
            LaunchWizardLaunchPath::ManualSetup => "Setup",
            LaunchWizardLaunchPath::FocusSession => "Focus",
        };
        // SPEC-2014 FR-126: rail の各ステップ状態を current_phase から導出する。
        let phase = self.current_phase();
        let is_manual = self.launch_path == LaunchWizardLaunchPath::ManualSetup;
        let setup_state = if !is_manual {
            "done"
        } else {
            match phase {
                WizardPhase::Path => "pending",
                WizardPhase::Settings => "active",
                WizardPhase::Runtime | WizardPhase::Confirm => "done",
            }
        };
        let runtime_state = if self.runtime_resolution_pending {
            "active"
        } else {
            match phase {
                WizardPhase::Runtime => "active",
                WizardPhase::Confirm => "done",
                WizardPhase::Settings => {
                    if self.runtime_context_resolved {
                        "done"
                    } else {
                        "pending"
                    }
                }
                WizardPhase::Path => {
                    if self.runtime_context_resolved
                        && self.launch_path != LaunchWizardLaunchPath::FocusSession
                    {
                        "done"
                    } else {
                        "pending"
                    }
                }
            }
        };
        let start_state = if self.launch_path == LaunchWizardLaunchPath::FocusSession
            || phase == WizardPhase::Confirm
        {
            "active"
        } else {
            "pending"
        };
        vec![
            LaunchWizardProgressStepView {
                key: "path".to_string(),
                label: path_label.to_string(),
                state: "done".to_string(),
                detail: None,
            },
            LaunchWizardProgressStepView {
                key: "setup".to_string(),
                label: "Settings".to_string(),
                state: setup_state.to_string(),
                detail: None,
            },
            LaunchWizardProgressStepView {
                key: "runtime".to_string(),
                label: "Runtime".to_string(),
                state: runtime_state.to_string(),
                detail: self.runtime_resolution_message.clone(),
            },
            LaunchWizardProgressStepView {
                key: "start".to_string(),
                label: "Start".to_string(),
                state: start_state.to_string(),
                detail: None,
            },
        ]
    }

    pub(super) fn current_options(&self) -> Vec<LaunchWizardOptionView> {
        match self.step {
            LaunchWizardStep::QuickStart => {
                let mut options = Vec::new();
                for (index, entry) in self.quick_start_entries.iter().enumerate() {
                    let summary = quick_start_summary(entry);
                    if let Some(reuse_action_label) = entry.reuse_action_label() {
                        options.push(LaunchWizardOptionView {
                            value: format!("reuse:{index}"),
                            label: format!("{reuse_action_label} {}", entry.tool_label),
                            description: Some(summary.clone()),
                            color: None,
                        });
                    }
                    options.push(LaunchWizardOptionView {
                        value: format!("start_new:{index}"),
                        label: format!("Start new with {}", entry.tool_label),
                        description: Some(summary),
                        color: None,
                    });
                }
                if self.latest_running_session().is_some() {
                    options.push(LaunchWizardOptionView {
                        value: "focus_existing".to_string(),
                        label: "Focus existing session".to_string(),
                        description: Some("Jump to a running window on this branch".to_string()),
                        color: None,
                    });
                }
                options.push(LaunchWizardOptionView {
                    value: "choose_different".to_string(),
                    label: "Choose different".to_string(),
                    description: Some("Open the full launch wizard".to_string()),
                    color: None,
                });
                options
            }
            LaunchWizardStep::FocusExistingSession => self
                .running_live_sessions()
                .map(|(_, entry)| LaunchWizardOptionView {
                    value: entry.window_id.clone(),
                    label: entry.name.clone(),
                    description: entry.detail.clone(),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::BranchAction => vec![
                LaunchWizardOptionView {
                    value: "use_selected".to_string(),
                    label: "Use selected branch".to_string(),
                    description: Some("Launch on the selected branch".to_string()),
                    color: None,
                },
                LaunchWizardOptionView {
                    value: "create_new".to_string(),
                    label: "Create new from selected".to_string(),
                    description: Some(
                        "Create a new branch based on the selected branch".to_string(),
                    ),
                    color: None,
                },
            ],
            LaunchWizardStep::BranchTypeSelect => BRANCH_TYPE_PREFIXES
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
                .collect(),
            LaunchWizardStep::LaunchTarget => launch_target_options_view(),
            LaunchWizardStep::AgentSelect => self
                .detected_agents
                .iter()
                .map(|agent| LaunchWizardOptionView {
                    value: agent.id.clone(),
                    label: agent.name.clone(),
                    description: Some(agent_description(agent)),
                    color: agent_option_color(&agent.id),
                })
                .collect(),
            LaunchWizardStep::ModelSelect => model_display_options(self.effective_agent_id())
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.label.to_string(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::ReasoningLevel => self
                .current_reasoning_options()
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.stored_value.to_string(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::RuntimeTarget => RUNTIME_TARGET_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.label.to_ascii_lowercase(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::WindowsShell => windows_shell_options_view(),
            LaunchWizardStep::DockerServiceSelect => self
                .docker_service_options()
                .into_iter()
                .map(|service| LaunchWizardOptionView {
                    value: service.clone(),
                    label: service,
                    description: Some("Docker Compose service".to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::DockerLifecycle => self
                .docker_lifecycle_options()
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: docker_lifecycle_value(option.intent).to_string(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::VersionSelect => self
                .current_version_options()
                .into_iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.value,
                    label: option.label,
                    description: Some("Tool version".to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::ExecutionMode => {
                execution_mode_options_view(self.current_agent_supports_resume_picker())
            }
            LaunchWizardStep::SkipPermissions => YES_NO_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.label.to_ascii_lowercase(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::CodexFastMode => FAST_MODE_OPTIONS
                .iter()
                .map(|option| LaunchWizardOptionView {
                    value: option.label.to_ascii_lowercase(),
                    label: option.label.to_string(),
                    description: Some(option.description.to_string()),
                    color: None,
                })
                .collect(),
            LaunchWizardStep::BranchNameInput => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use tempfile::tempdir;

    use super::super::profiles::load_launch_sessions;
    use super::super::test_support::*;
    use super::*;

    #[test]
    fn start_methods_view_exposes_direct_methods() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-live".to_string(),
            window_id: "tab-1:agent-live".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Running,
        }];
        let state = LaunchWizardState::open_with(
            ctx,
            sample_agent_options(),
            vec![quick_start_entry(
                "session-newer",
                "codex",
                Some("native-newer"),
                None,
                gwt_agent::LaunchRuntimeTarget::Docker,
                Some("gwt"),
            )],
        );

        let methods = state.view().start_methods;
        assert_eq!(methods.len(), 5);
        assert_eq!(methods[0].kind, "configure_and_start");
        assert_eq!(methods[0].label, "Configure and start");
        assert_eq!(methods[0].summary, "Edit settings before launch");
        assert_eq!(methods[1].kind, "start_with_last_settings");
        assert_eq!(methods[1].summary, "New session with saved settings");
        assert_eq!(methods[2].kind, "continue_last_session");
        assert_eq!(methods[2].badge, "Session");
        assert_eq!(methods[2].summary, "Resume conversation history");
        assert!(methods[2]
            .detail
            .as_deref()
            .unwrap_or("")
            .contains("native-newer"));
        assert_eq!(methods[3].kind, "open_session_picker");
        assert_eq!(methods[3].badge, "Picker");
        assert_eq!(methods[4].kind, "focus_running_session");
        assert_eq!(methods[4].badge, "Running");
        assert!(methods.iter().all(|method| method.enabled));
    }

    #[test]
    fn start_methods_use_concise_action_copy_without_repeating_settings() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![quick_start_entry(
                "session-newer",
                "codex",
                Some("native-newer"),
                None,
                gwt_agent::LaunchRuntimeTarget::Docker,
                Some("gwt"),
            )],
        );

        let methods = state.view().start_methods;
        let configure = methods
            .iter()
            .find(|method| method.kind == "configure_and_start")
            .expect("configure method");
        assert_eq!(configure.summary, "Edit settings before launch");
        assert!(configure.detail.is_none());

        let saved = methods
            .iter()
            .find(|method| method.kind == "start_with_last_settings")
            .expect("saved settings method");
        assert_eq!(saved.summary, "New session with saved settings");
        assert!(saved.detail.is_none());

        let continue_method = methods
            .iter()
            .find(|method| method.kind == "continue_last_session")
            .expect("continue method");
        assert_eq!(continue_method.summary, "Resume conversation history");

        let picker = methods
            .iter()
            .find(|method| method.kind == "open_session_picker")
            .expect("picker method");
        assert_eq!(picker.summary, "Choose a saved session");

        for method in methods
            .iter()
            .filter(|method| method.kind != "focus_running_session")
        {
            assert!(
                !method.summary.contains(" / "),
                "start method summary should not repeat launch settings: {:?}",
                method
            );
        }
    }

    #[test]
    fn disabled_start_methods_keep_reason_copy_non_redundant() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        let methods = state.view().start_methods;
        let saved = methods
            .iter()
            .find(|method| method.kind == "start_with_last_settings")
            .expect("saved settings method");
        assert_eq!(saved.summary, "Use saved launch settings");
        assert_eq!(saved.disabled_reason.as_deref(), Some("None saved yet"));

        let continue_method = methods
            .iter()
            .find(|method| method.kind == "continue_last_session")
            .expect("continue method");
        assert_eq!(continue_method.summary, "Resume recent session");
        assert_eq!(
            continue_method.disabled_reason.as_deref(),
            Some("No saved session")
        );

        let focus = methods
            .iter()
            .find(|method| method.kind == "focus_running_session")
            .expect("focus running method");
        assert_eq!(focus.summary, "Switch to running session");
        assert_eq!(focus.disabled_reason.as_deref(), Some("No running session"));
    }

    #[test]
    fn start_methods_view_marks_recommended_available_and_unavailable_groups() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        let view = serde_json::to_value(state.view()).expect("view json");
        let methods = view["start_methods"]
            .as_array()
            .expect("start methods array");

        let configure = methods
            .iter()
            .find(|method| method["kind"] == "configure_and_start")
            .expect("configure method");
        assert_eq!(configure["group"], "recommended");
        assert_eq!(configure["recommended"], true);

        let picker = methods
            .iter()
            .find(|method| method["kind"] == "open_session_picker")
            .expect("picker method");
        assert_eq!(picker["group"], "available");
        assert_eq!(picker["recommended"], false);

        for kind in [
            "start_with_last_settings",
            "continue_last_session",
            "focus_running_session",
        ] {
            let method = methods
                .iter()
                .find(|method| method["kind"] == kind)
                .expect("method by kind");
            assert_eq!(method["group"], "unavailable", "{kind}");
            assert_eq!(method["recommended"], false, "{kind}");
        }
    }

    #[test]
    fn start_methods_recommend_existing_running_work_before_resume_or_new_session() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-live".to_string(),
            window_id: "tab-1:agent-live".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Running Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Running,
        }];
        let state = LaunchWizardState::open_with(
            ctx,
            sample_agent_options(),
            vec![quick_start_entry(
                "session-newer",
                "codex",
                Some("native-newer"),
                None,
                gwt_agent::LaunchRuntimeTarget::Docker,
                Some("gwt"),
            )],
        );

        let view = serde_json::to_value(state.view()).expect("view json");
        let methods = view["start_methods"]
            .as_array()
            .expect("start methods array");

        let focus = methods
            .iter()
            .find(|method| method["kind"] == "focus_running_session")
            .expect("focus method");
        assert_eq!(focus["group"], "recommended");
        assert_eq!(focus["recommended"], true);

        for kind in [
            "configure_and_start",
            "start_with_last_settings",
            "continue_last_session",
            "open_session_picker",
        ] {
            let method = methods
                .iter()
                .find(|method| method["kind"] == kind)
                .expect("method by kind");
            assert_eq!(method["group"], "available", "{kind}");
            assert_eq!(method["recommended"], false, "{kind}");
        }
    }

    #[test]
    fn manual_setup_hides_execution_mode_and_launches_new_session() {
        let mut codex = sample_session_record(
            "feature/old",
            Path::new("/tmp/old-repo"),
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 6, 13, 9, 0, 0).unwrap(),
            None,
        );
        codex.session_mode = gwt_agent::SessionMode::Continue;
        let previous_profiles = previous_launch_profiles_from_sessions(&[codex]);
        let mut state = LaunchWizardState::open_with_previous_profiles(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
            previous_profiles,
        );

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        let view = state.view();
        assert!(!view.show_execution_mode);
        assert!(!view.launch_summary.iter().any(|item| item.label == "Mode"));
        assert_eq!(
            next_step(LaunchWizardStep::VersionSelect, &state),
            Some(LaunchWizardStep::SkipPermissions)
        );

        // Legacy protocol input may still arrive from an old frontend or a
        // persisted draft, but Manual setup is a new-session path.
        state.apply(LaunchWizardAction::SetExecutionMode {
            mode: "continue".to_string(),
        });
        assert_eq!(state.view().selected_execution_mode, "continue");
        state.apply(LaunchWizardAction::Submit); // Runtime -> Confirm
        assert!(state.view().show_confirm);
        state.apply(LaunchWizardAction::Submit); // Confirm -> Launch

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.session_mode, gwt_agent::SessionMode::Normal);
                    assert!(config.resume_session_id.is_none());
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected launch completion, got {other:?}"),
        }
    }

    #[test]
    fn start_methods_expose_session_picker_for_picker_capable_agents() {
        let state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        let methods = state.view().start_methods;
        assert!(methods.iter().any(|method| {
            method.kind == "open_session_picker"
                && method.label == "Open session picker"
                && method.enabled
        }));

        let unsupported = AgentOption {
            id: "proxy-agent".to_string(),
            name: "Proxy Agent".to_string(),
            available: true,
            installed_version: Some("1.0.0".to_string()),
            versions: Vec::new(),
            custom_agent: Some(sample_custom_agent(
                "proxy-agent",
                "Proxy Agent",
                gwt_agent::custom::CustomAgentType::Command,
                "proxy-agent",
            )),
        };
        let unsupported_state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            vec![unsupported],
            Vec::new(),
        );
        assert!(!unsupported_state
            .view()
            .start_methods
            .iter()
            .any(|method| method.kind == "open_session_picker"));
    }

    #[test]
    fn start_methods_focus_running_session_uses_latest_running_window() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![
            LiveSessionEntry {
                session_id: "session-idle".to_string(),
                window_id: "tab-1:agent-idle".to_string(),
                agent_id: "codex".to_string(),
                kind: "agent".to_string(),
                name: "Idle Codex".to_string(),
                detail: Some("/tmp/repo-idle".to_string()),
                active: true,
                runtime_status: crate::WindowProcessStatus::Idle,
            },
            LiveSessionEntry {
                session_id: "session-running".to_string(),
                window_id: "tab-1:agent-running".to_string(),
                agent_id: "codex".to_string(),
                kind: "agent".to_string(),
                name: "Running Codex".to_string(),
                detail: Some("/tmp/repo-running".to_string()),
                active: true,
                runtime_status: crate::WindowProcessStatus::Running,
            },
        ];
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        let view = state.view();
        let focus_method = view
            .start_methods
            .iter()
            .find(|method| method.kind == "focus_running_session")
            .expect("focus method");
        assert!(focus_method.enabled);
        assert_eq!(focus_method.summary, "Running Codex");

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::FocusRunningSession,
        });

        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::FocusWindow { window_id })
                if window_id == "tab-1:agent-running"
        ));
    }

    #[test]
    fn start_methods_focus_running_session_is_disabled_without_running_window() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-idle".to_string(),
            window_id: "tab-1:agent-idle".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Idle Codex".to_string(),
            detail: Some("/tmp/repo-idle".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Idle,
        }];
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        let view = state.view();
        let focus_method = view
            .start_methods
            .iter()
            .find(|method| method.kind == "focus_running_session")
            .expect("focus method");
        assert!(!focus_method.enabled);
        assert_eq!(focus_method.summary, "Switch to running session");

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::FocusRunningSession,
        });

        assert_eq!(
            state.error.as_deref(),
            Some("No running session is available")
        );
        assert!(state.completion.is_none());
    }

    #[test]
    fn start_methods_gate_setup_until_configure_is_selected() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![quick_start_entry(
                "session-newer",
                "codex",
                Some("native-newer"),
                None,
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
            )],
        );
        state.mark_runtime_context_unresolved();

        let initial = state.view();
        assert!(initial.show_start_methods);
        assert!(!initial.show_manual_setup);
        assert_eq!(initial.primary_action_label, "Choose start method");
        assert!(!initial.primary_action_enabled);

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });

        let configured = state.view();
        assert!(!configured.show_start_methods);
        assert!(configured.show_manual_setup);
        assert_eq!(configured.primary_action_label, "Continue");
        assert!(configured.primary_action_enabled);
    }

    #[test]
    fn back_from_configure_returns_to_start_methods_and_preserves_setup_draft() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![quick_start_entry(
                "session-newer",
                "codex",
                Some("native-newer"),
                None,
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
            )],
        );
        state.mark_runtime_context_unresolved();

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });
        state.apply(LaunchWizardAction::SetModel {
            model: "gpt-5.4".to_string(),
        });

        let configured = state.view();
        assert!(!configured.show_start_methods);
        assert!(configured.show_manual_setup);
        assert_eq!(configured.selected_model, "gpt-5.4");

        state.apply(LaunchWizardAction::Back);

        let backed = state.view();
        assert!(state.completion.is_none());
        assert!(backed.show_start_methods);
        assert!(!backed.show_manual_setup);
        assert_eq!(backed.selected_model, "gpt-5.4");

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        let configured_again = state.view();
        assert!(!configured_again.show_start_methods);
        assert!(configured_again.show_manual_setup);
        assert_eq!(configured_again.selected_model, "gpt-5.4");
    }

    #[test]
    fn start_with_last_settings_launches_new_session_without_resume_id() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![quick_start_entry(
                "session-newer",
                "codex",
                Some("native-newer"),
                None,
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
            )],
        );

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::StartWithLastSettings,
        });

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::ResolveRuntime(config))
            | Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.agent_id.command(), "codex");
                    assert_eq!(config.session_mode, gwt_agent::SessionMode::Normal);
                    assert_eq!(config.resume_session_id, None);
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected start method launch completion, got {other:?}"),
        }
    }

    #[test]
    fn start_with_last_settings_runtime_confirmation_stays_enabled_without_quick_start_entry() {
        let previous = LaunchWizardPreviousProfile {
            agent_id: "claude".to_string(),
            model: Some("Default (Opus 4.8)".to_string()),
            reasoning: Some("max".to_string()),
            version: Some("latest".to_string()),
            session_mode: gwt_agent::SessionMode::Normal,
            skip_permissions: true,
            codex_fast_mode: false,
            runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
            docker_service: None,
            docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            windows_shell: None,
        };
        let mut state = LaunchWizardState::open_start_work_with_previous_profile(
            context(branch("origin/develop"), "work/20260523-1406"),
            "origin/develop".to_string(),
            sample_agent_options(),
            Vec::new(),
            Some(previous.clone()),
        );
        state.mark_runtime_context_unresolved();

        let initial = state.view();
        assert!(initial.show_start_methods);
        assert!(initial
            .start_methods
            .iter()
            .any(|method| method.kind == "start_with_last_settings" && method.enabled));

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::StartWithLastSettings,
        });
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));

        state.completion = None;
        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "work/20260523-1406".to_string(),
            worktree_path: None,
            quick_start_root: PathBuf::from("/tmp/repo"),
            docker_context: None,
            docker_service_status: gwt_docker::ComposeServiceStatus::NotFound,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(LaunchWizardPreviousProfiles::from_profile(Some(previous))),
        });

        let confirmation = state.view();
        assert_eq!(confirmation.selected_launch_path, "manual_setup");
        assert!(confirmation.show_runtime_confirmation);
        assert_eq!(confirmation.primary_action_label, "Create and launch");
        assert!(confirmation.primary_action_enabled);

        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(config))
                if matches!(
                    config.as_ref(),
                    LaunchWizardLaunchRequest::Agent(config)
                        if config.branch.as_deref() == Some("work/20260523-1406")
                            && config.base_branch.as_deref() == Some("origin/develop")
                            && config.resume_session_id.is_none()
                )
        ));
    }

    #[test]
    fn panel_quick_start_resume_populates_launch_state() {
        let mut state = LaunchWizardState::open_with(
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
                runtime_target: gwt_agent::LaunchRuntimeTarget::Docker,
                docker_service: Some("gwt".to_string()),
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Restart,
            }],
        );

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });

        assert_eq!(state.agent_id, "codex");
        assert_eq!(state.model, "gpt-5.5");
        assert_eq!(state.reasoning, "high");
        assert_eq!(state.version, "0.110.0");
        assert_eq!(state.mode, "resume");
        assert_eq!(state.resume_session_id.as_deref(), Some("resume-1"));
        assert_eq!(state.runtime_target, gwt_agent::LaunchRuntimeTarget::Docker);
        assert_eq!(state.docker_service.as_deref(), Some("gwt"));
        assert!(state.skip_permissions);
        assert!(state.codex_fast_mode);
    }

    #[test]
    fn quick_start_with_removed_codex_model_falls_back_to_current_default() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "codex".to_string(),
                tool_label: "Codex".to_string(),
                model: Some("gpt-5.2-codex".to_string()),
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

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });

        assert_eq!(state.model, "gpt-5.5");
        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.model.as_deref(), Some("gpt-5.5"));
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected launch completion, got {other:?}"),
        }
    }

    #[test]
    fn quick_start_reuse_prefers_live_window_focus() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "gwt-session-1".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Running,
        }];

        let mut state = LaunchWizardState::open_with(
            ctx,
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

        let view = state.view();
        assert_eq!(
            view.quick_start_entries[0].reuse_action_label.as_deref(),
            Some("Focus")
        );

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::FocusWindow { window_id }) => {
                assert_eq!(window_id, "window-1");
            }
            other => panic!("expected focus completion, got {other:?}"),
        }
    }

    #[test]
    fn live_sessions_view_exposes_window_runtime_status() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "gwt-session-1".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Idle,
        }];

        let state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());
        let view = state.view();

        assert_eq!(view.live_sessions.len(), 1);
        assert_eq!(view.live_sessions[0].runtime_status, "idle");
    }

    #[test]
    fn quick_start_start_new_keeps_live_window_available_but_does_not_focus_it() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "gwt-session-1".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Running,
        }];

        let mut state = LaunchWizardState::open_with(
            ctx,
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

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::StartNew,
        });

        assert_eq!(state.mode, "normal");
        assert!(state.resume_session_id.is_none());
        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.session_mode, gwt_agent::SessionMode::Normal);
                    assert!(config.resume_session_id.is_none());
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected launch completion, got {other:?}"),
        }
    }

    #[test]
    fn quick_start_view_hides_reuse_action_without_live_or_saved_session() {
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
                resume_session_id: None,
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: true,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        let view = state.view();
        assert!(view.quick_start_entries[0].reuse_action_label.is_none());
    }

    #[test]
    fn panel_submit_requires_branch_name_for_new_branch() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );

        state.apply(LaunchWizardAction::SetBranchMode { create_new: true });
        state.apply(LaunchWizardAction::SetBranchName {
            value: String::new(),
        });
        state.apply(LaunchWizardAction::Submit);

        assert!(state.completion.is_none());
        assert_eq!(state.error.as_deref(), Some("Branch name is required"));
    }

    #[test]
    fn panel_view_exposes_selected_values_and_summary() {
        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        // SPEC-2014 FR-126: Settings フォーム項目は ConfigureAndStart の Settings
        // ステップ（未解決）で表示される。
        state.mark_runtime_context_unresolved();
        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        state.apply(LaunchWizardAction::SetAgent {
            agent_id: "codex".to_string(),
        });
        state.apply(LaunchWizardAction::SetModel {
            model: "gpt-5.5".to_string(),
        });
        state.apply(LaunchWizardAction::SetReasoning {
            reasoning: "high".to_string(),
        });
        state.apply(LaunchWizardAction::SetRuntimeTarget {
            target: gwt_agent::LaunchRuntimeTarget::Host,
        });
        state.apply(LaunchWizardAction::SetVersion {
            version: "0.110.0".to_string(),
        });
        state.apply(LaunchWizardAction::SetSkipPermissions { enabled: true });
        state.apply(LaunchWizardAction::SetCodexFastMode { enabled: true });

        let view = state.view();

        assert_eq!(view.branch_mode, "use_selected");
        assert_eq!(view.selected_agent_id, "codex");
        assert_eq!(view.selected_model, "gpt-5.5");
        assert_eq!(view.selected_reasoning, "high");
        assert_eq!(view.selected_runtime_target, "host");
        assert_eq!(view.selected_version, "0.110.0");
        assert!(view.show_reasoning);
        assert!(view.show_version);
        assert!(view.show_codex_fast_mode);
        assert!(view
            .launch_summary
            .iter()
            .any(|item| item.label == "Agent" && item.value == "Codex"));
        assert!(view
            .launch_summary
            .iter()
            .any(|item| item.label == "Fast mode" && item.value == "on"));
    }

    #[test]
    fn mutator_methods_validate_and_normalize_launch_options() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.set_branch_mode(true);
        assert!(state.is_new_branch);
        assert_eq!(state.base_branch_name.as_deref(), Some("feature/gui"));
        assert_eq!(state.branch_name, "feature/");

        state.branch_name = "feature/coverage".to_string();
        state.set_branch_type("bugfix/");
        assert_eq!(state.branch_name, "bugfix/coverage");
        state.set_branch_type("fix/");
        assert_eq!(state.error.as_deref(), Some("Branch type is unavailable"));

        state.mode = "resume".to_string();
        state.resume_session_id = Some("resume-1".to_string());
        state.skip_permissions = true;
        state.codex_fast_mode = true;
        state.set_launch_target(LaunchTargetKind::Shell);
        assert_eq!(state.mode, "normal");
        assert!(state.resume_session_id.is_none());
        assert!(!state.skip_permissions);
        assert!(!state.codex_fast_mode);

        state.set_launch_target(LaunchTargetKind::Agent);
        state.set_agent_id("codex");
        assert_eq!(state.agent_id, "codex");
        state.set_agent_id("missing");
        assert_eq!(state.error.as_deref(), Some("Agent option is unavailable"));

        state.set_model("gpt-5.5");
        assert_eq!(state.model, "gpt-5.5");
        state.set_model("bad-model");
        assert_eq!(state.error.as_deref(), Some("Model option is unavailable"));

        state.set_reasoning("high");
        assert_eq!(state.reasoning, "high");
        state.set_reasoning("extreme");
        assert_eq!(
            state.error.as_deref(),
            Some("Reasoning option is unavailable")
        );

        state.set_runtime_target(gwt_agent::LaunchRuntimeTarget::Docker);
        assert_eq!(state.runtime_target, gwt_agent::LaunchRuntimeTarget::Docker);
        assert_eq!(state.docker_service.as_deref(), Some("worker"));

        state.set_docker_service("api");
        assert_eq!(state.docker_service.as_deref(), Some("api"));
        state.set_docker_service("missing");
        assert_eq!(
            state.error.as_deref(),
            Some("Docker service is unavailable")
        );

        state.set_docker_lifecycle(gwt_agent::DockerLifecycleIntent::Connect);
        assert_eq!(
            state.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Connect
        );
        state.set_docker_lifecycle(gwt_agent::DockerLifecycleIntent::CreateAndStart);
        assert_eq!(
            state.error.as_deref(),
            Some("Docker lifecycle option is unavailable")
        );

        state.error = None;
        state.set_version("0.110.0");
        assert_eq!(state.version, "0.110.0");
        state.set_version("definitely-missing");
        assert_eq!(
            state.error.as_deref(),
            Some("Version option is unavailable")
        );

        state.resume_session_id = Some("resume-2".to_string());
        state.error = None;
        state.set_execution_mode("continue");
        assert_eq!(state.mode, "continue");
        assert!(state.resume_session_id.is_none());
        state.set_execution_mode("invalid");
        assert_eq!(
            state.error.as_deref(),
            Some("Execution mode is unavailable")
        );
    }

    #[test]
    fn private_selection_and_completion_helpers_cover_focus_and_submit_paths() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("api".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-1".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Running,
        }];
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());

        state.step = LaunchWizardStep::FocusExistingSession;
        state.selected = 0;
        state.apply_selection();
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::FocusWindow { window_id }) if window_id == "window-1"
        ));

        state.completion = None;
        state.selected = 9;
        state.apply_selection();
        assert_eq!(
            state.error.as_deref(),
            Some("No running session is available")
        );

        state.step = LaunchWizardStep::BranchAction;
        state.selected = 1;
        state.apply_selection();
        assert!(state.is_new_branch);

        state.step = LaunchWizardStep::BranchTypeSelect;
        state.selected = 1;
        state.apply_selection();
        assert!(state.branch_name.starts_with("bugfix/"));

        state.step = LaunchWizardStep::LaunchTarget;
        state.selected = 1;
        state.apply_selection();
        assert!(state.launch_target_is_shell());

        state.step = LaunchWizardStep::LaunchTarget;
        state.selected = 0;
        state.apply_selection();
        state.step = LaunchWizardStep::AgentSelect;
        state.selected = 1;
        state.apply_selection();
        assert_eq!(state.agent_id, "codex");

        state.step = LaunchWizardStep::ModelSelect;
        state.selected = 0;
        state.apply_selection();
        assert_eq!(state.model, "gpt-5.5");

        state.step = LaunchWizardStep::ReasoningLevel;
        state.selected = 1;
        state.apply_selection();
        assert!(!state.reasoning.is_empty());

        state.step = LaunchWizardStep::RuntimeTarget;
        state.selected = 1;
        state.apply_selection();
        assert_eq!(state.runtime_target, gwt_agent::LaunchRuntimeTarget::Docker);

        state.step = LaunchWizardStep::DockerServiceSelect;
        state.selected = 0;
        state.apply_selection();
        assert_eq!(state.docker_service.as_deref(), Some("api"));

        state.step = LaunchWizardStep::DockerLifecycle;
        state.selected = 0;
        state.apply_selection();

        state.step = LaunchWizardStep::VersionSelect;
        state.selected = 0;
        state.apply_selection();
        assert!(!state.version.is_empty());

        state.step = LaunchWizardStep::ExecutionMode;
        state.selected = 1;
        state.apply_selection();
        assert_eq!(state.mode, "continue");

        state.step = LaunchWizardStep::SkipPermissions;
        state.selected = 0;
        state.apply_selection();
        assert!(state.skip_permissions);

        state.step = LaunchWizardStep::CodexFastMode;
        state.selected = 0;
        state.apply_selection();
        assert!(state.codex_fast_mode);

        state.completion = None;
        state.step = LaunchWizardStep::CodexFastMode;
        state.advance_after_current_step();
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(_))
        ));

        state.completion = None;
        state.set_launch_target(LaunchTargetKind::Shell);
        state.submit_panel();
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(config))
                if matches!(config.as_ref(), LaunchWizardLaunchRequest::Shell(_))
        ));

        state.step = LaunchWizardStep::BranchNameInput;
        state.completion = None;
        state.error = None;
        state.apply(LaunchWizardAction::SubmitText {
            value: "  hotfix/coverage  ".to_string(),
        });
        assert_eq!(state.branch_name, "hotfix/coverage");
    }

    // SPEC-2014 Amendment 2026-05-20 (US-25 / FR-057 / SC-031)
    // Launch Wizard view should gate the "Linked issue" section so it only
    // appears when the wizard was opened through the Knowledge Issue Bridge
    // (`linked_issue_kind == Some(Issue)` AND `linked_issue_number.is_some()`).
    // SPEC Bridge, Active Work Add-Agent, Workspace Resume, and Branches paths
    // must hide the section.
    #[test]
    fn view_shows_linked_issue_only_for_issue_kind() {
        // Case 1: Knowledge Issue Bridge — kind=Issue + number=Some => true.
        let issue_state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Issue, 1938),
            sample_agent_options(),
            Vec::new(),
        );
        assert!(
            issue_state.view().show_linked_issue,
            "Issue Bridge (kind=Issue, number=Some) should set show_linked_issue=true"
        );
        assert_eq!(issue_state.view().linked_issue_number, Some(1938));

        // Case 2: Knowledge SPEC Bridge — kind=Spec + number=Some => false.
        let spec_state = LaunchWizardState::open_with(
            context_with_linked_issue(branch("develop"), "develop", LinkedIssueKind::Spec, 2014),
            sample_agent_options(),
            Vec::new(),
        );
        assert!(
            !spec_state.view().show_linked_issue,
            "SPEC Bridge (kind=Spec) should set show_linked_issue=false"
        );

        // Case 3: Active Work Add-Agent / Workspace Resume — kind=None + number=Some => false.
        let mut active_ctx = context(branch("develop"), "develop");
        active_ctx.linked_issue_number = Some(1234);
        let active_state =
            LaunchWizardState::open_with(active_ctx, sample_agent_options(), Vec::new());
        assert!(
            !active_state.view().show_linked_issue,
            "kind=None pre-fill (Active Work / Workspace Resume) should set show_linked_issue=false"
        );

        // Case 4: No number at all => false even for nominal Issue kind.
        let mut no_number_ctx = context(branch("develop"), "develop");
        no_number_ctx.linked_issue_kind = Some(LinkedIssueKind::Issue);
        let no_number_state =
            LaunchWizardState::open_with(no_number_ctx, sample_agent_options(), Vec::new());
        assert!(
            !no_number_state.view().show_linked_issue,
            "linked_issue_number=None should always set show_linked_issue=false"
        );

        // Case 5: Default empty context (Branches / direct launch) => false.
        let empty_state = LaunchWizardState::open_with(
            context(branch("develop"), "develop"),
            sample_agent_options(),
            Vec::new(),
        );
        assert!(
            !empty_state.view().show_linked_issue,
            "default context (Branches / direct launch) should set show_linked_issue=false"
        );
    }

    #[test]
    fn quick_start_resume_for_custom_agent_uses_config_backed_definition() {
        let dir = tempdir().expect("tempdir");
        let custom_path = dir.path().join("custom-agent");
        std::fs::write(&custom_path, "echo custom").expect("write custom agent stub");

        let mut state = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            build_agent_options(
                Vec::new(),
                &gwt_agent::VersionCache::new(),
                vec![sample_custom_agent(
                    "proxy-agent",
                    "Claude Proxy",
                    gwt_agent::custom::CustomAgentType::Path,
                    custom_path.display().to_string(),
                )],
            ),
            vec![QuickStartEntry {
                session_id: "gwt-session-1".to_string(),
                agent_id: "proxy-agent".to_string(),
                tool_label: "Claude Proxy".to_string(),
                model: None,
                reasoning: None,
                version: None,
                resume_session_id: Some("resume-1".to_string()),
                live_window_id: None,
                skip_permissions: true,
                codex_fast_mode: false,
                runtime_target: gwt_agent::LaunchRuntimeTarget::Host,
                docker_service: None,
                docker_lifecycle_intent: gwt_agent::DockerLifecycleIntent::Connect,
            }],
        );

        state.apply(LaunchWizardAction::ApplyQuickStart {
            index: 0,
            mode: QuickStartLaunchMode::Resume,
        });

        match state.completion.as_ref() {
            Some(LaunchWizardCompletion::Launch(config)) => match config.as_ref() {
                LaunchWizardLaunchRequest::Agent(config) => {
                    assert_eq!(config.command, custom_path.display().to_string());
                    assert_eq!(config.display_name, "Claude Proxy");
                    assert!(config.args.contains(&"--resume".to_string()));
                    assert!(config.args.contains(&"--unsafe".to_string()));
                }
                other => panic!("expected agent launch request, got {other:?}"),
            },
            other => panic!("expected quick start launch completion, got {other:?}"),
        }
    }

    #[test]
    fn phase_one_hides_runtime_until_worktree_is_resolved() {
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["app".to_string()],
            suggested_service: Some("app".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with_previous_profiles(
            ctx,
            sample_agent_options(),
            Vec::new(),
            Default::default(),
        );
        state.mark_runtime_context_unresolved();

        let view = state.view();
        assert!(!view.runtime_context_resolved);
        assert!(!view.show_runtime_target);
        assert!(!view.show_docker_service);
        assert!(!view.show_docker_lifecycle);

        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion,
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));

        state.completion = None;
        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/repo-feature-current")),
            quick_start_root: PathBuf::from("/tmp/repo-feature-current"),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(Default::default()),
        });

        let view = state.view();
        assert!(view.runtime_context_resolved);
        assert!(view.show_runtime_target);
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.selected_docker_service.as_deref(), Some("app"));
        assert!(
            view.progress_steps
                .iter()
                .any(|step| step.key == "runtime" && step.state == "active"),
            "Runtime confirmation must keep the Runtime rail step active",
        );
        assert!(
            view.progress_steps
                .iter()
                .any(|step| step.key == "start" && step.state == "pending"),
            "Start must stay pending while Runtime choices are still visible",
        );
    }

    #[test]
    fn back_from_runtime_confirmation_returns_to_settings_preserving_runtime_selection() {
        // SPEC-2014 US-37 / FR-123..FR-125 / SC-077, SC-078:
        // Runtime 確認画面で Back が表示され、押下すると Settings へ戻って
        // runtime target 等の選択が保持される。
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["app".to_string()],
            suggested_service: Some("app".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());
        state.mark_runtime_context_unresolved();

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion,
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));
        state.completion = None;

        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/repo-feature-current")),
            quick_start_root: PathBuf::from("/tmp/repo-feature-current"),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(Default::default()),
        });

        let view = state.view();
        assert!(
            view.show_runtime_confirmation,
            "setup path must reach the runtime confirmation screen",
        );
        assert_eq!(view.selected_runtime_target, "docker");
        // SC-077 / FR-123: Back must be visible on the runtime confirmation screen.
        assert!(
            view.show_back_button,
            "FR-123: Back must be visible on the runtime confirmation screen",
        );

        // FR-124: Back returns to the Settings form instead of being a no-op.
        state.apply(LaunchWizardAction::Back);
        let backed = state.view();
        assert!(
            state.completion.is_none(),
            "Back from runtime confirmation must not cancel the wizard",
        );
        assert!(
            backed.show_manual_setup,
            "FR-124: Back from runtime confirmation returns to the Settings form",
        );
        assert!(!backed.show_runtime_confirmation);
        // SC-078 / FR-125: runtime target selection is preserved across Back.
        assert_eq!(
            backed.selected_runtime_target, "docker",
            "FR-125: runtime target selection must be preserved on Back",
        );
    }

    #[test]
    fn manual_setup_runtime_step_advances_to_confirm_before_launch() {
        // SPEC-2014 US-38 / FR-127 / SC-081:
        // ManualSetup は Runtime ステップで Submit すると Confirm（読み取りサマリ）へ
        // 進み、Confirm で Submit して初めて Launch する。
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["app".to_string()],
            suggested_service: Some("app".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());
        state.mark_runtime_context_unresolved();

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion,
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));
        state.completion = None;

        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/repo-feature-current")),
            quick_start_root: PathBuf::from("/tmp/repo-feature-current"),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(Default::default()),
        });

        // Runtime ステップ: 編集 UI が見え、primary は Confirm へ進む "Continue"。
        let view = state.view();
        assert!(
            view.show_runtime_confirmation,
            "runtime step is shown after resolve"
        );
        assert!(!view.show_confirm, "confirm step is not active yet");
        assert_eq!(view.primary_action_label, "Continue");

        // Runtime → Submit → Confirm（Launch しない）。
        state.apply(LaunchWizardAction::Submit);
        assert!(
            state.completion.is_none(),
            "FR-127: advancing from runtime to confirm must not launch",
        );
        let view = state.view();
        assert!(
            view.show_confirm,
            "confirm step is shown after runtime submit"
        );
        assert!(
            !view.show_runtime_confirmation,
            "runtime edit UI is hidden on the confirm step",
        );
        assert!(
            !view.launch_summary.is_empty(),
            "confirm step shows the read-only launch summary",
        );
        assert_eq!(view.primary_action_label, "Launch");

        // Confirm → Submit → Launch（唯一の実起動点）。
        state.apply(LaunchWizardAction::Submit);
        assert!(
            matches!(state.completion, Some(LaunchWizardCompletion::Launch(_))),
            "FR-127: launch happens only from the confirm step",
        );

        // FR-124: Confirm から Back すると Runtime ステップへ戻る。
        state.completion = None;
        state.apply(LaunchWizardAction::Back);
        let view = state.view();
        assert!(
            view.show_runtime_confirmation,
            "FR-124: Back from confirm returns to the runtime step",
        );
        assert!(!view.show_confirm);
    }

    fn manual_setup_to_runtime_step(branch_name: &str) -> LaunchWizardState {
        // ConfigureAndStart で Settings → Runtime（解決済み）まで進めた state を返す。
        let mut ctx = context(branch(branch_name), branch_name);
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["app".to_string()],
            suggested_service: Some("app".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with(ctx, sample_agent_options(), Vec::new());
        state.mark_runtime_context_unresolved();
        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ConfigureAndStart,
        });
        state.apply(LaunchWizardAction::Submit);
        state.completion = None;
        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: branch_name.to_string(),
            worktree_path: Some(PathBuf::from("/tmp/repo-runtime")),
            quick_start_root: PathBuf::from("/tmp/repo-runtime"),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(Default::default()),
        });
        state
    }

    #[test]
    fn progress_rail_goto_jumps_between_setup_phases() {
        // SPEC-2014 FR-128 / SC-080: rail クリックで Settings/Runtime/Confirm を移動。
        let mut state = manual_setup_to_runtime_step("feature/current");
        assert_eq!(state.view().phase, WizardPhase::Runtime);
        assert_eq!(state.view().selected_runtime_target, "docker");

        // Runtime -> Confirm
        state.apply(LaunchWizardAction::GotoStep {
            phase: WizardPhase::Confirm,
        });
        assert_eq!(state.view().phase, WizardPhase::Confirm);
        assert!(state.view().show_confirm);

        // Confirm -> Settings（resolved 保持、選択保持）
        state.apply(LaunchWizardAction::GotoStep {
            phase: WizardPhase::Settings,
        });
        let view = state.view();
        assert_eq!(view.phase, WizardPhase::Settings);
        assert!(view.show_manual_setup);
        assert_eq!(view.selected_runtime_target, "docker");

        // Settings -> Runtime（branch 不変 → 再解決なし）
        state.apply(LaunchWizardAction::GotoStep {
            phase: WizardPhase::Runtime,
        });
        assert_eq!(state.view().phase, WizardPhase::Runtime);
        assert!(
            state.completion.is_none(),
            "SC-082: unchanged branch must not re-resolve on rail jump",
        );
    }

    #[test]
    fn settings_resubmit_reresolves_runtime_only_when_branch_changes() {
        // SPEC-2014 FR-128 / SC-082
        let mut state = manual_setup_to_runtime_step("feature/current");

        // Settings へ戻り、branch 不変のまま Submit → 再解決しない。
        state.apply(LaunchWizardAction::GotoStep {
            phase: WizardPhase::Settings,
        });
        assert!(state.view().show_manual_setup);
        state.completion = None;
        state.apply(LaunchWizardAction::Submit);
        assert!(
            state.completion.is_none(),
            "SC-082: unchanged branch must not re-resolve",
        );
        assert_eq!(state.view().phase, WizardPhase::Runtime);

        // Settings へ戻り branch を変更 → 再解決する。
        state.apply(LaunchWizardAction::GotoStep {
            phase: WizardPhase::Settings,
        });
        state.branch_name = "feature/other".to_string();
        state.completion = None;
        state.apply(LaunchWizardAction::Submit);
        assert!(
            matches!(
                state.completion,
                Some(LaunchWizardCompletion::ResolveRuntime(_))
            ),
            "SC-082: changed branch must re-resolve runtime",
        );
    }

    #[test]
    fn continue_last_session_start_method_skips_manual_settings_until_runtime_confirmation() {
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["app".to_string()],
            suggested_service: Some("app".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with_previous_profiles(
            ctx,
            sample_agent_options(),
            vec![quick_start_entry(
                "session-1",
                "codex",
                Some("resume-1"),
                None,
                gwt_agent::LaunchRuntimeTarget::Docker,
                Some("app"),
            )],
            Default::default(),
        );
        state.mark_runtime_context_unresolved();

        let view = state.view();
        assert_eq!(view.selected_launch_path, "quick_start");
        assert_eq!(view.selected_quick_start_index, Some(0));
        assert!(view.show_start_methods);
        assert!(!view.show_manual_setup);
        assert!(!view.show_runtime_confirmation);
        assert_eq!(view.primary_action_label, "Choose start method");

        state.apply(LaunchWizardAction::UseStartMethod {
            method: LaunchWizardStartMethodKind::ContinueLastSession,
        });
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::ResolveRuntime(config))
                if matches!(
                    config.as_ref(),
                    LaunchWizardLaunchRequest::Agent(config)
                        if config.resume_session_id.as_deref() == Some("resume-1")
                )
        ));

        state.completion = None;
        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/repo-feature-current")),
            quick_start_root: PathBuf::from("/tmp/repo-feature-current"),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: Vec::new(),
            previous_profiles: Some(Default::default()),
        });

        let view = state.view();
        assert_eq!(view.selected_launch_path, "quick_start");
        assert!(!view.show_manual_setup);
        assert!(view.show_runtime_confirmation);
        assert!(view.show_runtime_target);
        assert_eq!(view.selected_runtime_target, "docker");
        assert_eq!(view.primary_action_label, "Launch");
    }

    #[test]
    fn quick_start_runtime_confirmation_edit_is_preserved_on_launch() {
        let mut ctx = context(branch("feature/current"), "feature/current");
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["app".to_string()],
            suggested_service: Some("app".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let entry = quick_start_entry(
            "session-1",
            "codex",
            Some("resume-1"),
            None,
            gwt_agent::LaunchRuntimeTarget::Docker,
            Some("app"),
        );
        let mut state = LaunchWizardState::open_with_previous_profiles(
            ctx,
            sample_agent_options(),
            vec![entry.clone()],
            Default::default(),
        );
        state.mark_runtime_context_unresolved();

        state.apply(LaunchWizardAction::Submit);
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::ResolveRuntime(_))
        ));
        state.completion = None;

        state.apply_runtime_context(LaunchWizardHydration {
            selected_branch: None,
            normalized_branch_name: "feature/current".to_string(),
            worktree_path: Some(PathBuf::from("/tmp/repo-feature-current")),
            quick_start_root: PathBuf::from("/tmp/repo-feature-current"),
            docker_context: Some(DockerWizardContext {
                services: vec!["app".to_string()],
                suggested_service: Some("app".to_string()),
            }),
            docker_service_status: gwt_docker::ComposeServiceStatus::Running,
            agent_options: sample_agent_options(),
            quick_start_entries: vec![entry],
            previous_profiles: Some(Default::default()),
        });
        state.apply(LaunchWizardAction::SetRuntimeTarget {
            target: gwt_agent::LaunchRuntimeTarget::Host,
        });
        state.apply(LaunchWizardAction::Submit);

        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Launch(config))
                if matches!(
                    config.as_ref(),
                    LaunchWizardLaunchRequest::Agent(config)
                        if config.runtime_target == gwt_agent::LaunchRuntimeTarget::Host
                            && config.docker_service.is_none()
                            && config.resume_session_id.as_deref() == Some("resume-1")
                )
        ));
    }

    #[test]
    fn runtime_resolution_pending_updates_footer_and_progress() {
        let mut state = LaunchWizardState::open_with_previous_profiles(
            context(branch("feature/current"), "feature/current"),
            sample_agent_options(),
            Vec::new(),
            Default::default(),
        );
        state.mark_runtime_context_unresolved();
        state.mark_runtime_resolution_pending("Preparing worktree...");

        let view = state.view();
        assert!(view.runtime_resolution_pending);
        assert_eq!(
            view.runtime_resolution_message.as_deref(),
            Some("Preparing worktree...")
        );
        assert_eq!(view.primary_action_label, "Preparing...");
        assert!(!view.primary_action_enabled);
        assert!(view
            .progress_steps
            .iter()
            .any(|step| step.key == "runtime" && step.state == "active"));
    }

    #[test]
    fn quick_start_treats_codex_placeholder_resume_id_as_metadata_only() {
        let dir = tempdir().expect("tempdir");
        let worktree = dir.path().join("repo");
        std::fs::create_dir_all(&worktree).expect("repo dir");
        sample_session(
            dir.path(),
            "feature/gui",
            &worktree,
            gwt_agent::AgentId::Codex,
            Utc.with_ymd_and_hms(2026, 4, 14, 11, 0, 0).unwrap(),
            "agent-session",
        );

        let entries = quick_start_entries_from_sessions(
            &worktree,
            "feature/gui",
            &load_launch_sessions(dir.path()),
        );

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].resume_session_id, None);
        assert_eq!(entries[0].reuse_action_label(), None);
    }

    #[test]
    fn current_options_cover_all_steps_and_reasoning_variants() {
        let mut ctx = context(branch("feature/gui"), "feature/gui");
        ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-live".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Running,
        }];
        ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut state = LaunchWizardState::open_with(
            ctx,
            sample_agent_options(),
            vec![
                quick_start_entry(
                    "session-1",
                    "codex",
                    Some("resume-1"),
                    Some("window-1"),
                    gwt_agent::LaunchRuntimeTarget::Docker,
                    Some("worker"),
                ),
                quick_start_entry(
                    "session-2",
                    "claude",
                    None,
                    None,
                    gwt_agent::LaunchRuntimeTarget::Host,
                    None,
                ),
            ],
        );

        let quick_options = state.current_options();
        assert_eq!(quick_options[0].value, "reuse:0");
        assert!(quick_options
            .iter()
            .any(|option| option.value == "focus_existing"));
        assert!(quick_options
            .iter()
            .any(|option| option.value == "choose_different"));
        state.selected = 999;
        assert!(matches!(
            state.selected_quick_start_action(),
            QuickStartAction::ChooseDifferent
        ));
        assert!(state.selected_quick_start_entry().is_none());

        state.step = LaunchWizardStep::FocusExistingSession;
        assert_eq!(state.current_options()[0].value, "window-1");

        state.step = LaunchWizardStep::BranchAction;
        assert_eq!(state.current_options().len(), 2);

        state.step = LaunchWizardStep::BranchTypeSelect;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "release/"));

        state.step = LaunchWizardStep::LaunchTarget;
        assert_eq!(state.current_options()[1].value, "shell");

        state.step = LaunchWizardStep::AgentSelect;
        assert_eq!(state.current_options().len(), 2);

        state.agent_id = "claude".to_string();
        state.step = LaunchWizardStep::ModelSelect;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "sonnet"));

        state.model = "opus".to_string();
        state.step = LaunchWizardStep::ReasoningLevel;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "xhigh"));

        state.model = "sonnet".to_string();
        assert!(!state
            .current_options()
            .iter()
            .any(|option| option.value == "xhigh"));

        state.step = LaunchWizardStep::RuntimeTarget;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "docker"));

        state.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        state.step = LaunchWizardStep::DockerServiceSelect;
        assert_eq!(state.current_options()[1].value, "worker");

        state.step = LaunchWizardStep::DockerLifecycle;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "connect"));

        state.context.docker_service_status = gwt_docker::ComposeServiceStatus::Exited;
        assert_eq!(state.current_options()[0].value, "start");

        state.context.docker_service_status = gwt_docker::ComposeServiceStatus::NotFound;
        assert_eq!(state.current_options()[0].value, "create_and_start");

        state.context.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        state.agent_id = "missing".to_string();
        state.step = LaunchWizardStep::VersionSelect;
        assert!(state.current_options().is_empty());

        state.agent_id = "codex".to_string();
        state.version = "0.110.0".to_string();
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "0.110.0" || option.value == "latest"));

        state.step = LaunchWizardStep::ExecutionMode;
        assert!(state
            .current_options()
            .iter()
            .any(|option| option.value == "resume"));

        state.step = LaunchWizardStep::SkipPermissions;
        assert_eq!(state.current_options()[0].value, "yes");

        state.step = LaunchWizardStep::CodexFastMode;
        assert_eq!(state.current_options()[0].value, "on");

        state.step = LaunchWizardStep::BranchNameInput;
        assert!(state.current_options().is_empty());
    }

    #[test]
    fn navigation_and_apply_actions_cover_cancel_back_and_focus_paths() {
        let mut quick_ctx = context(branch("feature/gui"), "feature/gui");
        quick_ctx.live_sessions = vec![LiveSessionEntry {
            session_id: "session-live".to_string(),
            window_id: "window-1".to_string(),
            agent_id: "codex".to_string(),
            kind: "agent".to_string(),
            name: "Codex".to_string(),
            detail: Some("/tmp/repo".to_string()),
            active: true,
            runtime_status: crate::WindowProcessStatus::Running,
        }];
        let mut state = LaunchWizardState::open_with(
            quick_ctx,
            sample_agent_options(),
            vec![quick_start_entry(
                "session-1",
                "codex",
                Some("resume-1"),
                Some("window-1"),
                gwt_agent::LaunchRuntimeTarget::Host,
                None,
            )],
        );

        assert_eq!(
            next_step(LaunchWizardStep::QuickStart, &state),
            Some(LaunchWizardStep::SkipPermissions)
        );
        state.selected = 2;
        assert_eq!(
            next_step(LaunchWizardStep::QuickStart, &state),
            Some(LaunchWizardStep::FocusExistingSession)
        );
        state.selected = 3;
        assert_eq!(
            next_step(LaunchWizardStep::QuickStart, &state),
            Some(LaunchWizardStep::BranchAction)
        );
        assert_eq!(
            prev_step(LaunchWizardStep::BranchAction, &state),
            Some(LaunchWizardStep::QuickStart)
        );
        assert_eq!(
            prev_step(LaunchWizardStep::FocusExistingSession, &state),
            Some(LaunchWizardStep::QuickStart)
        );

        state.apply(LaunchWizardAction::FocusExistingSession { index: 0 });
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::FocusWindow { window_id }) if window_id == "window-1"
        ));

        state.completion = None;
        state.error = None;
        state.apply(LaunchWizardAction::FocusExistingSession { index: 99 });
        assert_eq!(
            state.error.as_deref(),
            Some("No running session is available")
        );

        state.completion = None;
        state.step = LaunchWizardStep::QuickStart;
        state.apply(LaunchWizardAction::Back);
        assert!(matches!(
            state.completion.as_ref(),
            Some(LaunchWizardCompletion::Cancelled)
        ));

        let mut plain = LaunchWizardState::open_with(
            context(branch("feature/gui"), "feature/gui"),
            sample_agent_options(),
            Vec::new(),
        );
        assert_eq!(
            next_step(LaunchWizardStep::LaunchTarget, &plain),
            Some(LaunchWizardStep::AgentSelect)
        );
        plain.launch_target = LaunchTargetKind::Shell;
        let expected_shell_step = if cfg!(windows) {
            Some(LaunchWizardStep::WindowsShell)
        } else {
            None
        };
        assert_eq!(
            next_step(LaunchWizardStep::LaunchTarget, &plain),
            expected_shell_step
        );
        plain.apply(LaunchWizardAction::SetLinkedIssue { issue_number: 42 });
        assert_eq!(plain.linked_issue_number, Some(42));
        plain.apply(LaunchWizardAction::ClearLinkedIssue);
        assert_eq!(plain.linked_issue_number, None);
        plain.step = LaunchWizardStep::BranchNameInput;
        plain.apply(LaunchWizardAction::SubmitText {
            value: "   ".to_string(),
        });
        assert_eq!(plain.error.as_deref(), Some("Branch name is required"));
        plain.error = None;
        plain.step = LaunchWizardStep::BranchAction;
        plain.apply(LaunchWizardAction::SubmitText {
            value: "ignored".to_string(),
        });
        assert!(plain.error.is_none());
        assert_eq!(plain.branch_name, "feature/gui");

        let mut docker_ctx = context(branch("feature/gui"), "feature/gui");
        docker_ctx.docker_context = Some(DockerWizardContext {
            services: vec!["api".to_string(), "worker".to_string()],
            suggested_service: Some("worker".to_string()),
        });
        docker_ctx.docker_service_status = gwt_docker::ComposeServiceStatus::Running;
        let mut docker =
            LaunchWizardState::open_with(docker_ctx, sample_agent_options(), Vec::new());
        docker.agent_id = "claude".to_string();
        docker.model = "sonnet".to_string();
        docker.runtime_target = gwt_agent::LaunchRuntimeTarget::Docker;
        assert_eq!(
            prev_step(LaunchWizardStep::RuntimeTarget, &docker),
            Some(LaunchWizardStep::ReasoningLevel)
        );
        assert_eq!(
            prev_step(LaunchWizardStep::DockerLifecycle, &docker),
            Some(LaunchWizardStep::DockerServiceSelect)
        );

        docker.apply(LaunchWizardAction::SetBranchType {
            prefix: "release/".to_string(),
        });
        assert!(docker.branch_name.starts_with("release/"));
        docker.apply(LaunchWizardAction::SetDockerService {
            service: "api".to_string(),
        });
        assert_eq!(docker.docker_service.as_deref(), Some("api"));
        docker.apply(LaunchWizardAction::SetDockerLifecycle {
            intent: gwt_agent::DockerLifecycleIntent::Restart,
        });
        assert_eq!(
            docker.docker_lifecycle_intent,
            gwt_agent::DockerLifecycleIntent::Restart
        );
        docker.apply(LaunchWizardAction::SetExecutionMode {
            mode: "continue".to_string(),
        });
        assert_eq!(docker.mode, "continue");
    }
}
